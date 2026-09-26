#include "mecore/engine.h"

#include "book/types.h"

#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <fcntl.h>
#include <limits>
#include <stdexcept>
#include <utility>
#include <unistd.h>

namespace sarvex::mecore {
namespace {

constexpr uint32_t kIoc = 1U << 0;
constexpr uint32_t kFok = 1U << 1;
constexpr uint32_t kPostOnly = 1U << 2;

bool is_buy(sarvex::v1::Side side, sarvex::v1::Action action) {
  const bool positive = side == sarvex::v1::SIDE_YES || side == sarvex::v1::SIDE_LONG;
  return positive == (action == sarvex::v1::ACTION_BUY);
}

void set_now(google::protobuf::Timestamp* timestamp) {
  const auto now = std::chrono::system_clock::now();
  const auto seconds = std::chrono::time_point_cast<std::chrono::seconds>(now);
  timestamp->set_seconds(seconds.time_since_epoch().count());
  timestamp->set_nanos(static_cast<int32_t>(
      std::chrono::duration_cast<std::chrono::nanoseconds>(now - seconds).count()));
}

}  // namespace

SarvaOrder::SarvaOrder(std::string order_id, std::string user_id, std::string hold_id,
                       std::string ticker, bool buy, sarvex::v1::Side side,
                       sarvex::v1::Action action, uint64_t price, uint64_t quantity)
    : order_id_(std::move(order_id)),
      user_id_(std::move(user_id)),
      hold_id_(std::move(hold_id)),
      ticker_(std::move(ticker)),
      buy_(buy),
      side_(side),
      action_(action),
      price_(price),
      quantity_(quantity) {}

class Engine::Observer final
    : public liquibook::book::OrderListener<SarvaOrder*>,
      public liquibook::book::TradeListener<
          liquibook::book::OrderBook<SarvaOrder*>>,
      public liquibook::book::DepthListener<Engine::Book> {
 public:
  explicit Observer(Engine* engine) : engine_(engine) {}

  void on_accept(SarvaOrder* const& order) override { engine_->on_accept(order); }
  void on_reject(SarvaOrder* const& order, const char* reason) override {
    engine_->on_reject(order, reason);
  }
  void on_fill(SarvaOrder* const& order, SarvaOrder* const& matched_order,
               liquibook::book::Quantity quantity,
               liquibook::book::Price price) override {
    engine_->on_fill(order, matched_order, quantity, price);
  }
  void on_cancel(SarvaOrder* const& order) override { engine_->on_cancel(order); }
  void on_cancel_reject(SarvaOrder* const& order, const char* reason) override {
    engine_->on_cancel_reject(order, reason);
  }
  void on_replace(SarvaOrder* const& order, const int64_t& size_delta,
                  liquibook::book::Price new_price) override {
    engine_->on_replace(order, size_delta, new_price);
  }
  void on_replace_reject(SarvaOrder* const& order, const char* reason) override {
    engine_->on_replace_reject(order, reason);
  }
  void on_trade(const liquibook::book::OrderBook<SarvaOrder*>*,
                liquibook::book::Quantity,
                liquibook::book::Price) override {}
  void on_depth_change(const Engine::Book* book, const Engine::DepthTracker* depth) override {
    engine_->on_depth_change(book, depth);
  }

 private:
  Engine* engine_;
};

Engine::Engine(std::size_t queue_capacity, std::chrono::milliseconds command_timeout)
    : queue_capacity_(queue_capacity), command_timeout_(command_timeout) {
  const char* configured = std::getenv("ME_CORE_JOURNAL_PATH");
  if (configured != nullptr && std::strlen(configured) != 0) {
    journal_path_ = configured;
    restore_journal();
    journal_.open(journal_path_, std::ios::binary | std::ios::app);
  }
  worker_ = std::thread(&Engine::run, this);
}

Engine::~Engine() { stop(); }

void Engine::stop() {
  {
    std::lock_guard lock(queue_mutex_);
    if (stopping_) return;
    stopping_ = true;
  }
  queue_cv_.notify_all();
  if (worker_.joinable()) worker_.join();
  std::vector<std::shared_ptr<Subscriber>> live_subscribers;
  {
    std::lock_guard lock(subscribers_mutex_);
    for (auto& subscriber : subscribers_) {
      if (auto live = subscriber.lock()) live_subscribers.push_back(std::move(live));
    }
    subscribers_.clear();
  }
  for (auto& subscriber : live_subscribers) {
    subscriber->close();
  }
}

void Engine::run() {
  for (;;) {
    std::function<void()> function;
    {
      std::unique_lock lock(queue_mutex_);
      queue_cv_.wait(lock, [this] { return stopping_ || !queue_.empty(); });
      if (stopping_ && queue_.empty()) return;
      function = std::move(queue_.front());
      queue_.pop_front();
    }
    function();
  }
}

template <typename T>
T Engine::dispatch(std::function<T()> function) {
  auto promise = std::make_shared<std::promise<T>>();
  auto future = promise->get_future();
  {
    std::lock_guard lock(queue_mutex_);
    if (stopping_ || queue_.size() >= queue_capacity_) {
      T result;
      result.queue_full = true;
      return result;
    }
    queue_.push_back([promise, function = std::move(function)]() mutable {
      try {
        promise->set_value(function());
      } catch (...) {
        // The gRPC boundary must never terminate the sequencer thread. The
        // command types have no exception transport, so return an unknown
        // outcome and force the caller to reconcile by order id.
        T result;
        result.unknown = true;
        promise->set_value(std::move(result));
      }
    });
  }
  queue_cv_.notify_one();
  if (future.wait_for(command_timeout_) != std::future_status::ready) {
    T result;
    result.unknown = true;
    return result;
  }
  return future.get();
}

AddBookResult Engine::add_book(const sarvex::v1::AddBookRequest& request) {
  return dispatch<AddBookResult>([this, request] {
    append_journal('A', request);
    return apply_add_book(request);
  });
}
CloseBookResult Engine::close_book(const sarvex::v1::CloseBookRequest& request) {
  return dispatch<CloseBookResult>([this, request] {
    append_journal('C', request);
    return apply_close_book(request);
  });
}
SubmitResult Engine::submit_order(const sarvex::v1::MeSubmitOrderRequest& request) {
  return dispatch<SubmitResult>([this, request] {
    append_journal('S', request);
    return apply_submit_order(request);
  });
}
CancelResult Engine::cancel_order(const sarvex::v1::MeCancelOrderRequest& request) {
  return dispatch<CancelResult>([this, request] {
    append_journal('X', request);
    return apply_cancel_order(request);
  });
}
AmendResult Engine::amend_order(const sarvex::v1::MeAmendOrderRequest& request) {
  return dispatch<AmendResult>([this, request] {
    append_journal('M', request);
    return apply_amend_order(request);
  });
}
SnapshotResult Engine::snapshot(const sarvex::v1::GetBookSnapshotRequest& request) {
  return dispatch<SnapshotResult>([this, request] { return apply_snapshot(request); });
}

void Engine::append_journal(char type, const google::protobuf::Message& command) {
  if (replaying_ || !journal_.is_open()) return;
  std::string bytes;
  if (!command.SerializeToString(&bytes) || bytes.size() > std::numeric_limits<uint32_t>::max()) {
    throw std::runtime_error("failed to serialize me-core journal command");
  }
  const auto length = static_cast<uint32_t>(bytes.size());
  journal_.write(&type, sizeof(type));
  journal_.write(reinterpret_cast<const char*>(&length), sizeof(length));
  journal_.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
  journal_.flush();
  if (!journal_) throw std::runtime_error("failed to append me-core journal");
  const char* fsync = std::getenv("ME_CORE_JOURNAL_FSYNC");
  if (fsync != nullptr && std::string(fsync) == "true") {
    const int fd = ::open(journal_path_.c_str(), O_WRONLY);
    if (fd < 0 || ::fsync(fd) != 0) {
      if (fd >= 0) ::close(fd);
      throw std::runtime_error("failed to fsync me-core journal");
    }
    ::close(fd);
  }
}

void Engine::restore_journal() {
  std::ifstream input(journal_path_, std::ios::binary);
  if (!input.good()) return;
  replaying_ = true;
  for (;;) {
    char type = 0;
    uint32_t length = 0;
    input.read(&type, sizeof(type));
    if (input.eof()) break;
    if (!input.read(reinterpret_cast<char*>(&length), sizeof(length)) || length > 64U * 1024U * 1024U) {
      throw std::runtime_error("corrupt me-core journal header");
    }
    std::string bytes(length, '\0');
    if (!input.read(bytes.data(), static_cast<std::streamsize>(length))) {
      throw std::runtime_error("truncated me-core journal");
    }
    switch (type) {
      case 'A': { sarvex::v1::AddBookRequest command; if (!command.ParseFromString(bytes)) throw std::runtime_error("invalid AddBook journal command"); apply_add_book(command); break; }
      case 'C': { sarvex::v1::CloseBookRequest command; if (!command.ParseFromString(bytes)) throw std::runtime_error("invalid CloseBook journal command"); apply_close_book(command); break; }
      case 'S': { sarvex::v1::MeSubmitOrderRequest command; if (!command.ParseFromString(bytes)) throw std::runtime_error("invalid Submit journal command"); apply_submit_order(command); break; }
      case 'X': { sarvex::v1::MeCancelOrderRequest command; if (!command.ParseFromString(bytes)) throw std::runtime_error("invalid Cancel journal command"); apply_cancel_order(command); break; }
      case 'M': { sarvex::v1::MeAmendOrderRequest command; if (!command.ParseFromString(bytes)) throw std::runtime_error("invalid Amend journal command"); apply_amend_order(command); break; }
      default: throw std::runtime_error("unknown me-core journal command");
    }
  }
  replaying_ = false;
}

Engine::BookState* Engine::find_book(const std::string& ticker) {
  const auto it = books_.find(ticker);
  return it == books_.end() ? nullptr : it->second.get();
}
const Engine::BookState* Engine::find_book(const std::string& ticker) const {
  const auto it = books_.find(ticker);
  return it == books_.end() ? nullptr : it->second.get();
}
SarvaOrder* Engine::find_order(const std::string& order_id) {
  const auto it = orders_.find(order_id);
  return it == orders_.end() ? nullptr : it->second.get();
}

AddBookResult Engine::apply_add_book(const sarvex::v1::AddBookRequest& request) {
  AddBookResult result;
  if (request.ticker().empty() || request.tick_size() <= 0 || request.min_price_ticks() < 0 ||
      request.max_price_ticks() < request.min_price_ticks()) {
    result.reject_code = "INVALID_BOOK";
    return result;
  }
  if (find_book(request.ticker()) != nullptr) {
    result.reject_code = "BOOK_ALREADY_EXISTS";
    return result;
  }
  auto state = std::make_unique<BookState>();
  state->ticker = request.ticker();
  state->kind = request.kind();
  state->tick_size = request.tick_size();
  state->min_price_ticks = request.min_price_ticks();
  state->max_price_ticks = request.max_price_ticks();
  state->book = std::make_unique<Book>(request.ticker());
  state->observer = std::make_unique<Observer>(this);
  state->book->set_order_listener(state->observer.get());
  state->book->set_trade_listener(state->observer.get());
  state->book->set_depth_listener(state->observer.get());
  books_.emplace(state->ticker, std::move(state));
  result.ok = true;
  return result;
}

CloseBookResult Engine::apply_close_book(const sarvex::v1::CloseBookRequest& request) {
  CloseBookResult result;
  result.ticker = request.ticker();
  auto* state = find_book(request.ticker());
  if (state == nullptr) {
    result.reject_code = "BOOK_NOT_FOUND";
    return result;
  }
  if (state->closed) {
    result.reject_code = "BOOK_ALREADY_CLOSED";
    return result;
  }
  state->closed = true;
  result.closed = true;
  result.close_global_seq = ++global_seq_;
  result.close_contract_seq = ++state->contract_seq;
  return result;
}

bool Engine::crosses(const BookState& state, const SarvaOrder& order) const {
  const auto& opposite = order.is_buy() ? state.book->asks() : state.book->bids();
  for (const auto& entry : opposite) {
    const auto price = entry.first.price();
    if (entry.second.open_qty() == 0) continue;
    if (order.price() == 0 || (order.is_buy() ? price <= order.price() : price >= order.price())) {
      return true;
    }
    break;
  }
  return false;
}

uint64_t Engine::available_quantity(const BookState& state, const SarvaOrder& order) const {
  uint64_t quantity = 0;
  const auto& opposite = order.is_buy() ? state.book->asks() : state.book->bids();
  for (const auto& entry : opposite) {
    const auto price = entry.first.price();
    if (entry.second.open_qty() == 0) continue;
    if (order.price() != 0 && (order.is_buy() ? price > order.price() : price < order.price())) break;
    if (entry.second.open_qty() > std::numeric_limits<uint64_t>::max() - quantity) {
      quantity = std::numeric_limits<uint64_t>::max();
    } else {
      quantity += entry.second.open_qty();
    }
  }
  return quantity;
}

std::string Engine::precheck(const BookState& state, const SarvaOrder& order,
                             uint32_t flags, sarvex::v1::SelfTradePreventionType stp) const {
  if (order.order_qty() == 0 || order.price() == 0) return "INVALID_PRICE_OR_SIZE";
  if (stp != sarvex::v1::SELF_TRADE_PREVENTION_TYPE_UNSPECIFIED && crosses(state, order)) {
    const auto& opposite = order.is_buy() ? state.book->asks() : state.book->bids();
    for (const auto& entry : opposite) {
      const auto price = entry.first.price();
      if (entry.second.open_qty() == 0) continue;
      if (order.price() != 0 && (order.is_buy() ? price > order.price() : price < order.price())) break;
      if (entry.second.ptr()->user_id() == order.user_id()) return "SELF_TRADE_PREVENTED";
    }
  }
  if ((flags & kPostOnly) != 0 && crosses(state, order)) return "POST_ONLY_WOULD_TRADE";
  if ((flags & kFok) != 0 && available_quantity(state, order) < order.order_qty()) {
    return "FOK_NOT_FILLED";
  }
  return {};
}

SubmitResult Engine::apply_submit_order(const sarvex::v1::MeSubmitOrderRequest& request) {
  SubmitResult result;
  result.order_id = request.order_id();
  if (request.order_id().empty() || request.user_id().empty() || request.ticker().empty()) {
    result.reject_code = "INVALID_ORDER";
    return result;
  }
  if (find_order(request.order_id()) != nullptr) {
    result.reject_code = "ORDER_ALREADY_EXISTS";
    return result;
  }
  auto* state = find_book(request.ticker());
  if (state == nullptr) {
    result.reject_code = "BOOK_NOT_FOUND";
    return result;
  }
  if (state->closed) {
    result.reject_code = "BOOK_CLOSED";
    return result;
  }
  if (request.count() <= 0 || request.price_ticks() <= 0 ||
      request.side() == sarvex::v1::SIDE_UNSPECIFIED ||
      request.action() == sarvex::v1::ACTION_UNSPECIFIED) {
    result.reject_code = "INVALID_ORDER";
    return result;
  }
  if (request.price_ticks() < state->min_price_ticks ||
      request.price_ticks() > state->max_price_ticks) {
    result.reject_code = "PRICE_OUT_OF_BOUNDS";
    return result;
  }
  auto order = std::make_unique<SarvaOrder>(
      request.order_id(), request.user_id(), request.hold_id(), request.ticker(),
      is_buy(request.side(), request.action()), request.side(), request.action(),
      static_cast<uint64_t>(request.price_ticks()), static_cast<uint64_t>(request.count()));
  const uint32_t flags = request.flags();
  order->set_conditions((flags & kFok) != 0, (flags & kIoc) != 0 || (flags & kFok) != 0);
  if (const auto reject = precheck(*state, *order, flags, request.stp()); !reject.empty()) {
    result.reject_code = reject;
    return result;
  }
  SarvaOrder* raw = order.get();
  orders_.emplace(request.order_id(), std::move(order));
  active_submit_ = &result;
  state->book->add(raw);
  active_submit_ = nullptr;
  if (!result.accepted && result.reject_code.empty()) result.reject_code = "ORDER_REJECTED";
  return result;
}

CancelResult Engine::apply_cancel_order(const sarvex::v1::MeCancelOrderRequest& request) {
  CancelResult result;
  result.order_id = request.order_id();
  auto* order = find_order(request.order_id());
  if (order == nullptr) {
    result.reject_code = "ORDER_NOT_FOUND";
    return result;
  }
  auto* state = find_book(order->ticker());
  if (state == nullptr) {
    result.reject_code = "BOOK_NOT_FOUND";
    return result;
  }
  result.cancelled_qty = static_cast<int64_t>(order->open_qty());
  active_cancel_ = &result;
  state->book->cancel(order);
  active_cancel_ = nullptr;
  if (!result.cancelled && result.reject_code.empty()) result.reject_code = "ORDER_NOT_OPEN";
  return result;
}

AmendResult Engine::apply_amend_order(const sarvex::v1::MeAmendOrderRequest& request) {
  AmendResult result;
  result.order_id = request.order_id();
  auto* order = find_order(request.order_id());
  if (order == nullptr) {
    result.reject_code = "ORDER_NOT_FOUND";
    return result;
  }
  auto* state = find_book(order->ticker());
  if (state == nullptr || state->closed) {
    result.reject_code = "BOOK_NOT_OPEN";
    return result;
  }
  if (request.new_price_ticks() <= 0 || request.new_count() <= 0 ||
      static_cast<uint64_t>(request.new_count()) < order->filled_qty()) {
    result.reject_code = "INVALID_AMEND";
    return result;
  }
  if (request.new_price_ticks() < state->min_price_ticks ||
      request.new_price_ticks() > state->max_price_ticks) {
    result.reject_code = "PRICE_OUT_OF_BOUNDS";
    return result;
  }
  if (order->filled_qty() != 0) {
    result.reject_code = "AMEND_PARTIAL_NOT_SUPPORTED";
    return result;
  }
  SarvaOrder candidate = *order;
  candidate.amend(static_cast<uint64_t>(request.new_price_ticks()),
                  static_cast<uint64_t>(request.new_count()));
  if (crosses(*state, candidate)) {
    result.reject_code = "AMEND_MATCH_NOT_SUPPORTED";
    return result;
  }
  const auto delta = static_cast<int64_t>(request.new_count()) -
                     static_cast<int64_t>(order->order_qty());
  active_amend_ = &result;
  state->book->cancel(order);
  if (!result.reject_code.empty()) {
    active_amend_ = nullptr;
    return result;
  }
  order->amend(static_cast<uint64_t>(request.new_price_ticks()),
               static_cast<uint64_t>(request.new_count()));
  state->book->add(order);
  active_amend_ = nullptr;
  if (result.reject_code.empty()) {
    result.amended = true;
    auto event = sarvex::v1::ExecutionEvent();
    event.set_ticker(order->ticker());
    event.set_global_seq(++global_seq_);
    event.set_contract_seq(++state->contract_seq);
    set_now(event.mutable_ts());
    auto* amended = event.mutable_amended();
    amended->set_order_id(order->order_id());
    amended->set_new_price_ticks(request.new_price_ticks());
    amended->set_new_count(request.new_count());
    publish(std::move(event));
  }
  (void)delta;
  return result;
}

SnapshotResult Engine::apply_snapshot(const sarvex::v1::GetBookSnapshotRequest& request) {
  SnapshotResult result;
  const auto* state = find_book(request.ticker());
  if (state == nullptr) return result;
  result.found = true;
  auto& snapshot = result.snapshot;
  snapshot.set_ticker(request.ticker());
  snapshot.set_seq(state->contract_seq);
  snapshot.set_book_seq(state->contract_seq);
  set_now(snapshot.mutable_ts());
  const int limit = request.depth() <= 0 ? 25 : std::min(request.depth(), 25);
  int count = 0;
  for (auto entry = state->book->bids().begin(); entry != state->book->bids().end();) {
    const auto price = entry->first.price();
    int64_t quantity = 0;
    int32_t order_count = 0;
    while (entry != state->book->bids().end() && entry->first.price() == price) {
      quantity += static_cast<int64_t>(entry->second.open_qty());
      ++order_count;
      ++entry;
    }
    if (quantity == 0) continue;
    if (count++ >= limit) break;
    auto* level = snapshot.add_bids();
    level->set_price_ticks(static_cast<int64_t>(price));
    level->set_total_qty(quantity);
    level->set_order_count(order_count);
  }
  count = 0;
  for (auto entry = state->book->asks().begin(); entry != state->book->asks().end();) {
    const auto price = entry->first.price();
    int64_t quantity = 0;
    int32_t order_count = 0;
    while (entry != state->book->asks().end() && entry->first.price() == price) {
      quantity += static_cast<int64_t>(entry->second.open_qty());
      ++order_count;
      ++entry;
    }
    if (quantity == 0) continue;
    if (count++ >= limit) break;
    auto* level = snapshot.add_asks();
    level->set_price_ticks(static_cast<int64_t>(price));
    level->set_total_qty(quantity);
    level->set_order_count(order_count);
  }
  return result;
}

uint64_t Engine::next_sequence(BookState& state) {
  ++global_seq_;
  ++state.contract_seq;
  return global_seq_;
}

void Engine::publish(sarvex::v1::ExecutionEvent event) {
  {
    std::lock_guard lock(subscribers_mutex_);
    history_.push_back(event);
    while (history_.size() > 4096) history_.pop_front();
    for (auto it = subscribers_.begin(); it != subscribers_.end();) {
      if (auto subscriber = it->lock()) {
        {
          std::lock_guard subscriber_lock(subscriber->mutex_);
          subscriber->events_.push_back(event);
          while (subscriber->events_.size() > 4096) subscriber->events_.pop_front();
        }
        subscriber->cv_.notify_one();
        ++it;
      } else {
        it = subscribers_.erase(it);
      }
    }
  }
}

void Engine::on_accept(SarvaOrder* order) {
  auto* state = find_book(order->ticker());
  if (state == nullptr) return;
  if (active_amend_ != nullptr) return;
  auto event = sarvex::v1::ExecutionEvent();
  event.set_ticker(order->ticker());
  event.set_global_seq(next_sequence(*state));
  event.set_contract_seq(state->contract_seq);
  set_now(event.mutable_ts());
  event.mutable_accepted()->set_order_id(order->order_id());
  publish(std::move(event));
  if (active_submit_ != nullptr && active_submit_->order_id == order->order_id()) {
    active_submit_->accepted = true;
    active_submit_->global_seq = global_seq_;
    active_submit_->contract_seq = state->contract_seq;
  }
}

void Engine::on_reject(SarvaOrder* order, const char* reason) {
  auto* state = find_book(order->ticker());
  if (state == nullptr) return;
  const std::string code = reason == nullptr ? "ORDER_REJECTED" : reason;
  auto event = sarvex::v1::ExecutionEvent();
  event.set_ticker(order->ticker());
  event.set_global_seq(next_sequence(*state));
  event.set_contract_seq(state->contract_seq);
  set_now(event.mutable_ts());
  auto* rejected = event.mutable_rejected();
  rejected->set_order_id(order->order_id());
  rejected->set_reject_code(code);
  publish(std::move(event));
  if (active_submit_ != nullptr && active_submit_->order_id == order->order_id()) {
    active_submit_->reject_code = code;
    active_submit_->global_seq = global_seq_;
    active_submit_->contract_seq = state->contract_seq;
  }
}

void Engine::on_fill(SarvaOrder* order, SarvaOrder* matched_order,
                     uint64_t quantity, uint64_t price) {
  auto* state = find_book(order->ticker());
  if (state == nullptr) return;
  order->add_fill(quantity);
  matched_order->add_fill(quantity);
  auto event = sarvex::v1::ExecutionEvent();
  event.set_ticker(order->ticker());
  event.set_global_seq(next_sequence(*state));
  event.set_contract_seq(state->contract_seq);
  set_now(event.mutable_ts());
  auto* fill = event.mutable_fill();
  fill->set_fill_id("fill_" + std::to_string(global_seq_));
  fill->set_maker_order_id(matched_order->order_id());
  fill->set_taker_order_id(order->order_id());
  fill->set_maker_user_id(matched_order->user_id());
  fill->set_taker_user_id(order->user_id());
  fill->set_price_ticks(static_cast<int64_t>(price));
  fill->set_count(static_cast<int64_t>(quantity));
  fill->set_aggressor_side(order->side());
  fill->set_ticker(order->ticker());
  fill->set_global_seq(global_seq_);
  fill->set_contract_seq(state->contract_seq);
  fill->mutable_ts()->CopyFrom(event.ts());
  fill->set_maker_hold_id(matched_order->hold_id());
  fill->set_taker_hold_id(order->hold_id());
  fill->set_maker_side(matched_order->side());
  fill->set_maker_action(matched_order->action());
  fill->set_taker_side(order->side());
  fill->set_taker_action(order->action());
  fill->set_maker_fee_micro_usdc(0);
  fill->set_taker_fee_micro_usdc(0);
  if (active_submit_ != nullptr && active_submit_->order_id == order->order_id()) {
    active_submit_->fills.push_back(*fill);
    active_submit_->global_seq = global_seq_;
    active_submit_->contract_seq = state->contract_seq;
  }
  publish(std::move(event));
}

void Engine::on_cancel(SarvaOrder* order) {
  if (active_amend_ != nullptr) return;
  auto* state = find_book(order->ticker());
  if (state == nullptr) return;
  auto event = sarvex::v1::ExecutionEvent();
  event.set_ticker(order->ticker());
  event.set_global_seq(next_sequence(*state));
  event.set_contract_seq(state->contract_seq);
  set_now(event.mutable_ts());
  auto* cancelled = event.mutable_cancelled();
  cancelled->set_order_id(order->order_id());
  cancelled->set_cancelled_qty(static_cast<int64_t>(order->open_qty()));
  cancelled->set_reason_code(active_submit_ == nullptr ? "USER_CANCEL" : "IOC_REMAINDER");
  publish(std::move(event));
  if (active_cancel_ != nullptr && active_cancel_->order_id == order->order_id()) {
    active_cancel_->cancelled = true;
  }
}

void Engine::on_cancel_reject(SarvaOrder* order, const char* reason) {
  const std::string code = reason == nullptr ? "ORDER_NOT_OPEN" : reason;
  if (active_amend_ != nullptr) {
    active_amend_->reject_code = code;
  } else if (active_cancel_ != nullptr) {
    active_cancel_->reject_code = code;
  }
}

void Engine::on_replace(SarvaOrder*, int64_t, uint64_t) {}
void Engine::on_replace_reject(SarvaOrder*, const char*) {}

void Engine::on_depth_change(const Book* book, const DepthTracker* depth) {
  auto* state = find_book(book->symbol());
  if (state == nullptr) return;
  std::map<int64_t, std::pair<int64_t, int32_t>, std::greater<>> bids;
  std::map<int64_t, std::pair<int64_t, int32_t>> asks;
  for (auto entry = book->bids().begin(); entry != book->bids().end();) {
    const auto price = static_cast<int64_t>(entry->first.price());
    auto& level = bids[price];
    while (entry != book->bids().end() && entry->first.price() == static_cast<uint64_t>(price)) {
      level.first += static_cast<int64_t>(entry->second.open_qty());
      ++level.second;
      ++entry;
    }
  }
  for (auto entry = book->asks().begin(); entry != book->asks().end();) {
    const auto price = static_cast<int64_t>(entry->first.price());
    auto& level = asks[price];
    while (entry != book->asks().end() && entry->first.price() == static_cast<uint64_t>(price)) {
      level.first += static_cast<int64_t>(entry->second.open_qty());
      ++level.second;
      ++entry;
    }
  }
  auto emit_levels = [this, state](const auto& previous, const auto& current,
                                   sarvex::v1::Side side,
                                   sarvex::v1::BookSide book_side) {
    std::map<int64_t, std::pair<int64_t, int32_t>> prices;
    for (const auto& item : previous) prices[item.first] = item.second;
    for (const auto& item : current) prices[item.first] = item.second;
    for (const auto& item : prices) {
      const auto old_it = previous.find(item.first);
      const auto new_it = current.find(item.first);
      const int64_t old_qty = old_it == previous.end() ? 0 : old_it->second.first;
      const int64_t new_qty = new_it == current.end() ? 0 : new_it->second.first;
      if (old_qty == new_qty) continue;
      auto event = sarvex::v1::ExecutionEvent();
      event.set_ticker(state->ticker);
      event.set_global_seq(next_sequence(*state));
      event.set_contract_seq(state->contract_seq);
      set_now(event.mutable_ts());
      auto* delta = event.mutable_book_delta();
      delta->set_side(side);
      delta->set_book_side(book_side);
      delta->set_book_seq(state->contract_seq);
      delta->set_price_ticks(item.first);
      delta->set_qty_delta(new_qty - old_qty);
      delta->set_new_total_qty(new_qty);
      delta->set_new_order_count(new_it == current.end() ? 0 : new_it->second.second);
      publish(std::move(event));
    }
  };
  (void)depth;
  const auto bid_side = state->kind == sarvex::v1::CONTRACT_KIND_BINARY
                            ? sarvex::v1::SIDE_YES
                            : sarvex::v1::SIDE_LONG;
  const auto ask_side = state->kind == sarvex::v1::CONTRACT_KIND_BINARY
                            ? sarvex::v1::SIDE_NO
                            : sarvex::v1::SIDE_SHORT;
  emit_levels(state->previous_bids, bids, bid_side, sarvex::v1::BOOK_SIDE_BID);
  emit_levels(state->previous_asks, asks, ask_side, sarvex::v1::BOOK_SIDE_ASK);
  state->previous_bids = std::move(bids);
  state->previous_asks = std::move(asks);
}

std::shared_ptr<Engine::Subscriber> Engine::subscribe(uint64_t from_global_seq) {
  auto subscriber = std::make_shared<Subscriber>();
  std::lock_guard lock(subscribers_mutex_);
  for (const auto& event : history_) {
    if (event.global_seq() > from_global_seq) subscriber->events_.push_back(event);
  }
  subscribers_.push_back(subscriber);
  return subscriber;
}

void Engine::unsubscribe(const std::shared_ptr<Subscriber>& subscriber) {
  {
    std::lock_guard lock(subscribers_mutex_);
    subscribers_.erase(std::remove_if(subscribers_.begin(), subscribers_.end(),
                                      [&subscriber](const auto& item) {
                                        return item.expired() || item.lock() == subscriber;
                                      }),
                       subscribers_.end());
  }
  subscriber->close();
}

bool Engine::Subscriber::pop(sarvex::v1::ExecutionEvent* event,
                             const std::function<bool()>& cancelled) {
  std::unique_lock lock(mutex_);
  cv_.wait_for(lock, std::chrono::milliseconds(250), [&] {
    return closed_ || !events_.empty() || cancelled();
  });
  if (events_.empty()) return !closed_ && !cancelled();
  *event = std::move(events_.front());
  events_.pop_front();
  return true;
}

void Engine::Subscriber::close() {
  {
    std::lock_guard lock(mutex_);
    closed_ = true;
  }
  cv_.notify_all();
}

}  // namespace sarvex::mecore
