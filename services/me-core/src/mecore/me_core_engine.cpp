#include "mecore/me_core_engine.h"

#include <algorithm>

namespace sarvex::me {

bool MeCoreEngine::add_book(const std::string& ticker,
                            ContractKind kind,
                            int64_t tick_size,
                            int64_t min_price_ticks,
                            int64_t max_price_ticks,
                            std::string* reject_reason) {
  if (ticker.empty()) {
    if (reject_reason != nullptr) {
      *reject_reason = "ticker is required";
    }
    return false;
  }
  if (tick_size <= 0 || min_price_ticks <= 0 || max_price_ticks <= 0 || min_price_ticks >= max_price_ticks) {
    if (reject_reason != nullptr) {
      *reject_reason = "invalid tick/price bounds";
    }
    return false;
  }
  if (kind != CONTRACT_KIND_BINARY && kind != CONTRACT_KIND_SCALAR) {
    if (reject_reason != nullptr) {
      *reject_reason = "unsupported contract kind";
    }
    return false;
  }
  if (state_.books_by_ticker.find(ticker) != state_.books_by_ticker.end()) {
    if (reject_reason != nullptr) {
      *reject_reason = "book already exists";
    }
    return false;
  }

  auto book_state = std::make_unique<BookState>(ticker);
  book_state->book->set_order_listener(state_.listener.get());
  book_state->book->set_trade_listener(state_.listener.get());
  book_state->book->set_depth_listener(state_.listener.get());

  state_.books_by_ticker.emplace(ticker, std::move(book_state));
  book_meta_[ticker] = BookMeta{kind, tick_size, min_price_ticks, max_price_ticks};

  return true;
}

std::optional<BookSnapshotView> MeCoreEngine::get_book_snapshot(const std::string& ticker, int32_t depth) const {
  auto it = state_.books_by_ticker.find(ticker);
  if (it == state_.books_by_ticker.end()) {
    return std::nullopt;
  }

  BookSnapshotView snap;
  snap.ticker = ticker;
  snap.contract_seq = it->second->contract_seq;

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

}  // namespace sarvex::me
