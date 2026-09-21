#pragma once

#include "sarvex/v1/match.pb.h"
#include "book/depth_order_book.h"
#include "book/depth_listener.h"
#include "book/order_listener.h"
#include "book/trade_listener.h"

#include <condition_variable>
#include <chrono>
#include <cstdint>
#include <deque>
#include <fstream>
#include <functional>
#include <future>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

namespace sarvex::mecore {

class SarvaOrder {
 public:
  SarvaOrder(std::string order_id, std::string user_id, std::string hold_id,
             std::string ticker, bool buy, sarvex::v1::Side side,
             sarvex::v1::Action action, uint64_t price, uint64_t quantity);

  bool is_limit() const { return price_ != 0; }
  bool is_buy() const { return buy_; }
  liquibook::book::Price price() const { return price_; }
  liquibook::book::Price stop_price() const { return 0; }
  liquibook::book::Quantity order_qty() const { return quantity_; }
  bool all_or_none() const { return all_or_none_; }
  bool immediate_or_cancel() const { return immediate_or_cancel_; }

  const std::string& order_id() const { return order_id_; }
  const std::string& user_id() const { return user_id_; }
  const std::string& hold_id() const { return hold_id_; }
  const std::string& ticker() const { return ticker_; }
  sarvex::v1::Side side() const { return side_; }
  sarvex::v1::Action action() const { return action_; }
  uint64_t filled_qty() const { return filled_qty_; }
  uint64_t open_qty() const { return quantity_ - filled_qty_; }

  void set_conditions(bool all_or_none, bool immediate_or_cancel) {
    all_or_none_ = all_or_none;
    immediate_or_cancel_ = immediate_or_cancel;
  }
  void add_fill(uint64_t quantity) { filled_qty_ += quantity; }
  void amend(uint64_t price, uint64_t quantity) {
    price_ = price;
    quantity_ = quantity;
  }

 private:
  std::string order_id_;
  std::string user_id_;
  std::string hold_id_;
  std::string ticker_;
  bool buy_;
  sarvex::v1::Side side_;
  sarvex::v1::Action action_;
  uint64_t price_;
  uint64_t quantity_;
  uint64_t filled_qty_{0};
  bool all_or_none_{false};
  bool immediate_or_cancel_{false};
};

struct CommandMeta {
  bool queue_full{false};
  bool unknown{false};
};

struct AddBookResult : CommandMeta {
  bool ok{false};
  std::string reject_code;
};

struct CloseBookResult : CommandMeta {
  bool closed{false};
  std::string ticker;
  std::string reject_code;
  uint64_t close_global_seq{0};
  uint64_t close_contract_seq{0};
};

struct SubmitResult : CommandMeta {
  bool accepted{false};
  std::string order_id;
  std::string reject_code;
  std::vector<sarvex::v1::MeFill> fills;
  uint64_t contract_seq{0};
  uint64_t global_seq{0};
};

struct CancelResult : CommandMeta {
  bool cancelled{false};
  std::string order_id;
  std::string reject_code;
  int64_t cancelled_qty{0};
};

struct AmendResult : CommandMeta {
  bool amended{false};
  std::string order_id;
  std::string reject_code;
};

struct SnapshotResult : CommandMeta {
  bool found{false};
  sarvex::v1::BookSnapshot snapshot;
};

class Engine {
 public:
  using Book = liquibook::book::DepthOrderBook<SarvaOrder*, 25>;
  using DepthTracker = Book::DepthTracker;

  explicit Engine(std::size_t queue_capacity = 4096,
                  std::chrono::milliseconds command_timeout =
                      std::chrono::milliseconds(100));
  ~Engine();

  Engine(const Engine&) = delete;
  Engine& operator=(const Engine&) = delete;

  AddBookResult add_book(const sarvex::v1::AddBookRequest& request);
  CloseBookResult close_book(const sarvex::v1::CloseBookRequest& request);
  SubmitResult submit_order(const sarvex::v1::MeSubmitOrderRequest& request);
  CancelResult cancel_order(const sarvex::v1::MeCancelOrderRequest& request);
  AmendResult amend_order(const sarvex::v1::MeAmendOrderRequest& request);
  SnapshotResult snapshot(const sarvex::v1::GetBookSnapshotRequest& request);

  class Subscriber {
   public:
    bool pop(sarvex::v1::ExecutionEvent* event,
             const std::function<bool()>& cancelled);
    void close();

   private:
    friend class Engine;
    std::mutex mutex_;
    std::condition_variable cv_;
    std::deque<sarvex::v1::ExecutionEvent> events_;
    bool closed_{false};
  };

  std::shared_ptr<Subscriber> subscribe(uint64_t from_global_seq);
  void unsubscribe(const std::shared_ptr<Subscriber>& subscriber);

 private:
  class Observer;
  struct BookState {
    std::string ticker;
    sarvex::v1::ContractKind kind;
    int64_t tick_size;
    int64_t min_price_ticks;
    int64_t max_price_ticks;
    std::unique_ptr<Book> book;
    std::unique_ptr<Observer> observer;
    uint64_t contract_seq{0};
    bool closed{false};
    std::map<int64_t, std::pair<int64_t, int32_t>, std::greater<>> previous_bids;
    std::map<int64_t, std::pair<int64_t, int32_t>> previous_asks;
  };

  template <typename T>
  T dispatch(std::function<T()> function);

  void run();
  void stop();
  void publish(sarvex::v1::ExecutionEvent event);
  uint64_t next_sequence(BookState& state);
  BookState* find_book(const std::string& ticker);
  const BookState* find_book(const std::string& ticker) const;
  SarvaOrder* find_order(const std::string& order_id);

  AddBookResult apply_add_book(const sarvex::v1::AddBookRequest& request);
  CloseBookResult apply_close_book(const sarvex::v1::CloseBookRequest& request);
  SubmitResult apply_submit_order(const sarvex::v1::MeSubmitOrderRequest& request);
  CancelResult apply_cancel_order(const sarvex::v1::MeCancelOrderRequest& request);
  AmendResult apply_amend_order(const sarvex::v1::MeAmendOrderRequest& request);
  SnapshotResult apply_snapshot(const sarvex::v1::GetBookSnapshotRequest& request);

  bool crosses(const BookState& state, const SarvaOrder& order) const;
  uint64_t available_quantity(const BookState& state, const SarvaOrder& order) const;
  std::string precheck(const BookState& state, const SarvaOrder& order,
                       uint32_t flags, sarvex::v1::SelfTradePreventionType stp) const;
  void on_accept(SarvaOrder* order);
  void on_reject(SarvaOrder* order, const char* reason);
  void on_fill(SarvaOrder* order, SarvaOrder* matched_order,
               uint64_t quantity, uint64_t price);
  void on_cancel(SarvaOrder* order);
  void on_cancel_reject(SarvaOrder* order, const char* reason);
  void on_replace(SarvaOrder* order, int64_t size_delta, uint64_t new_price);
  void on_replace_reject(SarvaOrder* order, const char* reason);
  void on_depth_change(const Book* book, const DepthTracker* depth);
  void restore_journal();
  void append_journal(char type, const google::protobuf::Message& command);
  bool journal_enabled() const { return journal_.is_open(); }

  std::size_t queue_capacity_;
  std::chrono::milliseconds command_timeout_;
  std::mutex queue_mutex_;
  std::condition_variable queue_cv_;
  std::deque<std::function<void()>> queue_;
  bool stopping_{false};
  std::thread worker_;
  std::string journal_path_;
  std::ofstream journal_;
  bool replaying_{false};

  std::map<std::string, std::unique_ptr<BookState>> books_;
  std::unordered_map<std::string, std::unique_ptr<SarvaOrder>> orders_;
  uint64_t global_seq_{0};
  std::deque<sarvex::v1::ExecutionEvent> history_;
  std::mutex subscribers_mutex_;
  std::vector<std::weak_ptr<Subscriber>> subscribers_;

  SubmitResult* active_submit_{nullptr};
  CancelResult* active_cancel_{nullptr};
  AmendResult* active_amend_{nullptr};
};

}  // namespace sarvex::mecore
