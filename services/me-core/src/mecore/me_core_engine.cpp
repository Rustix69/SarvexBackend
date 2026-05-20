#include "mecore/me_core_engine.h"

#include <algorithm>
#include <chrono>
#include <stdexcept>

namespace sarvex::me {

MeCoreEngine::MeCoreEngine() {
  sequencer_thread_ = std::thread(&MeCoreEngine::sequencer_loop, this);
}

MeCoreEngine::~MeCoreEngine() {
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    stopping_ = true;
  }
  queue_cv_.notify_all();
  if (sequencer_thread_.joinable()) {
    sequencer_thread_.join();
  }
}

bool MeCoreEngine::add_book(const std::string& ticker,
                            ContractKind kind,
                            int64_t tick_size,
                            int64_t min_price_ticks,
                            int64_t max_price_ticks,
                            std::string* reject_reason) {
  Command cmd;
  cmd.payload = AddBookCommand{ticker, kind, tick_size, min_price_ticks, max_price_ticks};
  auto fut = cmd.promise.get_future();
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    queue_.push_back(std::move(cmd));
  }
  queue_cv_.notify_one();
  auto result = fut.get();
  const bool ok = std::get<bool>(result);
  if (!ok && reject_reason != nullptr) {
    *reject_reason = "add_book rejected";
  }
  return ok;
}

SubmitOrderResultView MeCoreEngine::submit_order(const SubmitOrderRequestView& req) {
  Command cmd;
  cmd.payload = SubmitOrderCommand{req};
  auto fut = cmd.promise.get_future();
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    queue_.push_back(std::move(cmd));
  }
  queue_cv_.notify_one();
  return std::get<SubmitOrderResultView>(fut.get());
}

CancelOrderResultView MeCoreEngine::cancel_order(const std::string& ticker, const std::string& order_id) {
  Command cmd;
  cmd.payload = CancelOrderCommand{ticker, order_id};
  auto fut = cmd.promise.get_future();
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    queue_.push_back(std::move(cmd));
  }
  queue_cv_.notify_one();
  return std::get<CancelOrderResultView>(fut.get());
}

CloseBookResultView MeCoreEngine::close_book(const std::string& ticker) {
  Command cmd;
  cmd.payload = CloseBookCommand{ticker};
  auto fut = cmd.promise.get_future();
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    queue_.push_back(std::move(cmd));
  }
  queue_cv_.notify_one();
  return std::get<CloseBookResultView>(fut.get());
}

std::optional<BookSnapshotView> MeCoreEngine::get_book_snapshot(const std::string& ticker, int32_t depth) {
  Command cmd;
  cmd.payload = SnapshotCommand{ticker, depth};
  auto fut = cmd.promise.get_future();
  {
    std::lock_guard<std::mutex> lk(queue_mu_);
    queue_.push_back(std::move(cmd));
  }
  queue_cv_.notify_one();
  return std::get<std::optional<BookSnapshotView>>(fut.get());
}

void MeCoreEngine::sequencer_loop() {
  while (true) {
    Command cmd;
    {
      std::unique_lock<std::mutex> lk(queue_mu_);
      queue_cv_.wait(lk, [&]() { return stopping_ || !queue_.empty(); });
      if (stopping_ && queue_.empty()) {
        return;
      }
      cmd = std::move(queue_.front());
      queue_.pop_front();
    }
    process_command(std::move(cmd));
  }
}

void MeCoreEngine::process_command(Command&& cmd) {
  std::lock_guard<std::mutex> state_lk(state_mu_);

  if (std::holds_alternative<AddBookCommand>(cmd.payload)) {
    std::string reject_reason;
    bool ok = apply_add_book(std::get<AddBookCommand>(cmd.payload), &reject_reason);
    cmd.promise.set_value(ok);
    return;
  }
  if (std::holds_alternative<SubmitOrderCommand>(cmd.payload)) {
    cmd.promise.set_value(apply_submit_order(std::get<SubmitOrderCommand>(cmd.payload)));
    return;
  }
  if (std::holds_alternative<CancelOrderCommand>(cmd.payload)) {
    cmd.promise.set_value(apply_cancel_order(std::get<CancelOrderCommand>(cmd.payload)));
    return;
  }
  if (std::holds_alternative<CloseBookCommand>(cmd.payload)) {
    cmd.promise.set_value(apply_close_book(std::get<CloseBookCommand>(cmd.payload)));
    return;
  }
  if (std::holds_alternative<SnapshotCommand>(cmd.payload)) {
    cmd.promise.set_value(apply_snapshot(std::get<SnapshotCommand>(cmd.payload)));
    return;
  }
  cmd.promise.set_value(false);
}

bool MeCoreEngine::apply_add_book(const AddBookCommand& cmd, std::string* reject_reason) {
  if (cmd.ticker.empty()) {
    if (reject_reason != nullptr) {
      *reject_reason = "ticker is required";
    }
    return false;
  }
  if (cmd.tick_size <= 0 || cmd.min_price_ticks <= 0 || cmd.max_price_ticks <= 0 ||
      cmd.min_price_ticks >= cmd.max_price_ticks) {
    if (reject_reason != nullptr) {
      *reject_reason = "invalid tick/price bounds";
    }
    return false;
  }
  if (cmd.kind != CONTRACT_KIND_BINARY && cmd.kind != CONTRACT_KIND_SCALAR) {
    if (reject_reason != nullptr) {
      *reject_reason = "unsupported contract kind";
    }
    return false;
  }
  if (state_.books_by_ticker.find(cmd.ticker) != state_.books_by_ticker.end()) {
    if (reject_reason != nullptr) {
      *reject_reason = "book already exists";
    }
    return false;
  }

  auto book_state = std::make_unique<BookState>(cmd.ticker);
  book_state->book->set_order_listener(state_.listener.get());
  book_state->book->set_trade_listener(state_.listener.get());
  book_state->book->set_depth_listener(state_.listener.get());

  state_.books_by_ticker.emplace(cmd.ticker, std::move(book_state));
  book_meta_[cmd.ticker] = BookMeta{cmd.kind, cmd.tick_size, cmd.min_price_ticks, cmd.max_price_ticks};
  return true;
}

SubmitOrderResultView MeCoreEngine::apply_submit_order(const SubmitOrderCommand& cmd) {
  SubmitOrderResultView out;
  const auto& req = cmd.req;

  const auto bs_it = state_.books_by_ticker.find(req.ticker);
  if (bs_it == state_.books_by_ticker.end()) {
    out.reject_code = "BOOK_NOT_FOUND";
    return out;
  }
  auto& bs = *bs_it->second;
  if (bs.closed) {
    out.reject_code = "BOOK_CLOSED";
    return out;
  }

  const auto meta_it = book_meta_.find(req.ticker);
  if (meta_it == book_meta_.end()) {
    out.reject_code = "BOOK_META_NOT_FOUND";
    return out;
  }
  const auto& meta = meta_it->second;

  if (req.qty <= 0) {
    out.reject_code = "INVALID_QTY";
    return out;
  }
  if (req.price_ticks <= 0) {
    out.reject_code = "INVALID_PRICE";
    return out;
  }
  if (req.price_ticks < meta.min_price_ticks || req.price_ticks > meta.max_price_ticks) {
    out.reject_code = "PRICE_OUT_OF_RANGE";
    return out;
  }
  if (req.price_ticks % meta.tick_size != 0) {
    out.reject_code = "BAD_TICK_ALIGNMENT";
    return out;
  }

  if (req.post_only && would_cross(bs, req.is_buy, req.price_ticks)) {
    out.reject_code = "POST_ONLY_WOULD_MATCH";
    return out;
  }

  state_.global_seq++;
  bs.contract_seq++;
  out.global_seq = state_.global_seq;
  out.contract_seq = bs.contract_seq;

  const bool is_ioc = req.ioc;
  const bool is_aon = req.aon || req.fok;
  auto ord = std::make_unique<SarvaOrder>(req.order_id,
                                          req.user_id,
                                          req.hold_id,
                                          req.ticker,
                                          static_cast<liquibook::book::Quantity>(req.qty),
                                          static_cast<liquibook::book::Price>(req.price_ticks),
                                          req.is_buy,
                                          is_ioc,
                                          is_aon,
                                          0);
  auto* ord_ptr = ord.get();
  state_.orders_by_id[req.order_id] = std::move(ord);

  const auto event_start = state_.listener_events.size();
  const bool has_fill = bs.book->add(ord_ptr);
  (void)has_fill;

  std::size_t fill_idx = 0;
  for (std::size_t i = event_start; i < state_.listener_events.size(); ++i) {
    const auto& ev = state_.listener_events[i];
    if (ev.kind != "fill") {
      continue;
    }
    FillView fv;
    fv.fill_id = next_fill_id(req.ticker, bs.contract_seq, static_cast<uint64_t>(fill_idx++));
    fv.taker_order_id = ev.order_id;
    fv.maker_order_id = ev.matched_order_id;
    fv.price_ticks = static_cast<int64_t>(ev.price);
    fv.qty = static_cast<int64_t>(ev.qty);
    fv.global_seq = out.global_seq;
    fv.contract_seq = out.contract_seq;
    out.fills.push_back(std::move(fv));
  }

  if (req.fok && out.fills.empty()) {
    // FOK: must fully fill or no mutation. Liquibook AON prevents partial fills.
    bs.book->cancel(ord_ptr);
    state_.orders_by_id.erase(req.order_id);
    out.accepted = false;
    out.reject_code = "FOK_NOT_FILLED";
    return out;
  }

  out.accepted = true;
  return out;
}

CancelOrderResultView MeCoreEngine::apply_cancel_order(const CancelOrderCommand& cmd) {
  CancelOrderResultView out;
  const auto bs_it = state_.books_by_ticker.find(cmd.ticker);
  if (bs_it == state_.books_by_ticker.end()) {
    out.reject_code = "BOOK_NOT_FOUND";
    return out;
  }

  const auto ord_it = state_.orders_by_id.find(cmd.order_id);
  if (ord_it == state_.orders_by_id.end()) {
    out.reject_code = "ORDER_NOT_FOUND";
    return out;
  }

  state_.global_seq++;
  bs_it->second->contract_seq++;
  out.global_seq = state_.global_seq;
  out.contract_seq = bs_it->second->contract_seq;

  bs_it->second->book->cancel(ord_it->second.get());
  out.cancelled = true;
  return out;
}

CloseBookResultView MeCoreEngine::apply_close_book(const CloseBookCommand& cmd) {
  CloseBookResultView out;
  const auto bs_it = state_.books_by_ticker.find(cmd.ticker);
  if (bs_it == state_.books_by_ticker.end()) {
    out.reject_code = "BOOK_NOT_FOUND";
    return out;
  }
  if (bs_it->second->closed) {
    out.closed = true;
    out.close_global_seq = state_.global_seq;
    out.close_contract_seq = bs_it->second->contract_seq;
    return out;
  }

  state_.global_seq++;
  bs_it->second->contract_seq++;
  bs_it->second->closed = true;

  out.closed = true;
  out.close_global_seq = state_.global_seq;
  out.close_contract_seq = bs_it->second->contract_seq;
  return out;
}

std::optional<BookSnapshotView> MeCoreEngine::apply_snapshot(const SnapshotCommand& cmd) const {
  auto it = state_.books_by_ticker.find(cmd.ticker);
  if (it == state_.books_by_ticker.end()) {
    return std::nullopt;
  }

  BookSnapshotView snap;
  snap.ticker = cmd.ticker;
  snap.contract_seq = it->second->contract_seq;

  int32_t depth = cmd.depth;
  if (depth <= 0) {
    depth = 10;
  }

  int32_t n = 0;
  for (const auto& level : it->second->book->bids()) {
    if (n++ >= depth) {
      break;
    }
    snap.bids.push_back(
        PriceLevelView{static_cast<int64_t>(level.first.price()),
                       static_cast<int64_t>(level.second.open_qty())});
  }

  n = 0;
  for (const auto& level : it->second->book->asks()) {
    if (n++ >= depth) {
      break;
    }
    snap.asks.push_back(
        PriceLevelView{static_cast<int64_t>(level.first.price()),
                       static_cast<int64_t>(level.second.open_qty())});
  }

  return snap;
}

bool MeCoreEngine::would_cross(const BookState& bs, bool is_buy, int64_t price_ticks) const {
  if (is_buy) {
    const auto& asks = bs.book->asks();
    if (asks.empty()) {
      return false;
    }
    return static_cast<int64_t>(asks.begin()->first.price()) <= price_ticks;
  }

  const auto& bids = bs.book->bids();
  if (bids.empty()) {
    return false;
  }
  return static_cast<int64_t>(bids.begin()->first.price()) >= price_ticks;
}

std::string MeCoreEngine::next_fill_id(const std::string& ticker, uint64_t contract_seq, uint64_t idx) const {
  return ticker + ":" + std::to_string(contract_seq) + ":" + std::to_string(idx);
}

}  // namespace sarvex::me
