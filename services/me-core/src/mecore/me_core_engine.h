#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include "mecore/shard_state.h"
#include "mecore/proto_compat.h"

namespace sarvex::me {

struct PriceLevelView {
  int64_t price_ticks{0};
  int64_t total_qty{0};
};

struct BookSnapshotView {
  std::string ticker;
  uint64_t contract_seq{0};
  std::vector<PriceLevelView> bids;
  std::vector<PriceLevelView> asks;
};

class MeCoreEngine {
public:
  MeCoreEngine() = default;

  bool add_book(const std::string& ticker,
                ContractKind kind,
                int64_t tick_size,
                int64_t min_price_ticks,
                int64_t max_price_ticks,
                std::string* reject_reason);

  std::optional<BookSnapshotView> get_book_snapshot(const std::string& ticker, int32_t depth) const;

  ShardState& state() { return state_; }
  const ShardState& state() const { return state_; }

private:
  struct BookMeta {
    ContractKind kind{CONTRACT_KIND_UNSPECIFIED};
    int64_t tick_size{0};
    int64_t min_price_ticks{0};
    int64_t max_price_ticks{0};
  };

  std::unordered_map<std::string, BookMeta> book_meta_;
  ShardState state_;
};

}  // namespace sarvex::me
