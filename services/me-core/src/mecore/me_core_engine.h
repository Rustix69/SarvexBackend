#pragma once

#include <cstdint>
#include <deque>
#include <future>
#include <mutex>
#include <optional>
#include <condition_variable>
#include <string>
#include <thread>
#include <variant>
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

struct FillView {
  std::string fill_id;
  std::string maker_order_id;
  std::string taker_order_id;
  int64_t price_ticks{0};
  int64_t qty{0};
  uint64_t global_seq{0};
  uint64_t contract_seq{0};
};

struct SubmitOrderRequestView {
  std::string order_id;
  std::string user_id;
  std::string hold_id;
  std::string ticker;
  int64_t price_ticks{0};
  int64_t qty{0};
  bool is_buy{true};
  bool ioc{false};
  bool fok{false};
  bool post_only{false};
  bool aon{false};
};

struct SubmitOrderResultView {
  bool accepted{false};
  std::string reject_code;
  uint64_t global_seq{0};
  uint64_t contract_seq{0};
  std::vector<FillView> fills;
};

struct CancelOrderResultView {
  bool cancelled{false};
  std::string reject_code;
  uint64_t global_seq{0};
  uint64_t contract_seq{0};
};

struct CloseBookResultView {
  bool closed{false};
  std::string reject_code;
  uint64_t close_global_seq{0};
  uint64_t close_contract_seq{0};
};

class MeCoreEngine {
public:
  MeCoreEngine();
  ~MeCoreEngine();

  bool add_book(const std::string& ticker,
                ContractKind kind,
                int64_t tick_size,
                int64_t min_price_ticks,
                int64_t max_price_ticks,
                std::string* reject_reason);

  SubmitOrderResultView submit_order(const SubmitOrderRequestView& req);
  CancelOrderResultView cancel_order(const std::string& ticker, const std::string& order_id);
  CloseBookResultView close_book(const std::string& ticker);
  std::optional<BookSnapshotView> get_book_snapshot(const std::string& ticker, int32_t depth);

  ShardState& state() { return state_; }
  const ShardState& state() const { return state_; }

private:
  struct AddBookCommand {
    std::string ticker;
    ContractKind kind{CONTRACT_KIND_UNSPECIFIED};
    int64_t tick_size{0};
    int64_t min_price_ticks{0};
    int64_t max_price_ticks{0};
  };
  struct SubmitOrderCommand {
    SubmitOrderRequestView req;
  };
  struct CancelOrderCommand {
    std::string ticker;
    std::string order_id;
  };
  struct CloseBookCommand {
    std::string ticker;
  };
  struct SnapshotCommand {
    std::string ticker;
    int32_t depth{10};
  };

  using CommandPayload =
      std::variant<std::monostate, AddBookCommand, SubmitOrderCommand, CancelOrderCommand, CloseBookCommand, SnapshotCommand>;
  using CommandResult = std::variant<bool, SubmitOrderResultView, CancelOrderResultView, CloseBookResultView, std::optional<BookSnapshotView>>;

  struct Command {
    CommandPayload payload;
    std::promise<CommandResult> promise;
  };

  struct BookMeta {
    ContractKind kind{CONTRACT_KIND_UNSPECIFIED};
    int64_t tick_size{0};
    int64_t min_price_ticks{0};
    int64_t max_price_ticks{0};
  };

  mutable std::mutex state_mu_;
  std::mutex queue_mu_;
  std::condition_variable queue_cv_;
  std::deque<Command> queue_;
  std::thread sequencer_thread_;
  bool stopping_{false};

  std::unordered_map<std::string, BookMeta> book_meta_;
  ShardState state_;

  void sequencer_loop();
  void process_command(Command&& cmd);
  bool apply_add_book(const AddBookCommand& cmd, std::string* reject_reason);
  SubmitOrderResultView apply_submit_order(const SubmitOrderCommand& cmd);
  CancelOrderResultView apply_cancel_order(const CancelOrderCommand& cmd);
  CloseBookResultView apply_close_book(const CloseBookCommand& cmd);
  std::optional<BookSnapshotView> apply_snapshot(const SnapshotCommand& cmd) const;

  bool would_cross(const BookState& bs, bool is_buy, int64_t price_ticks) const;
  std::string next_fill_id(const std::string& ticker, uint64_t contract_seq, uint64_t idx) const;
};

}  // namespace sarvex::me
