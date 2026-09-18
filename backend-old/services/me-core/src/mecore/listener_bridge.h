#pragma once

#include <book/depth_listener.h>
#include <book/order_listener.h>
#include <book/order_book.h>
#include <book/trade_listener.h>

#include <string>
#include <vector>

#include "mecore/book_state.h"

namespace sarvex::me {

struct ListenerEvent {
  std::string kind;
  std::string order_id;
  std::string matched_order_id;
  liquibook::book::Quantity qty{0};
  liquibook::book::Price price{0};
};

class ListenerBridge final : public liquibook::book::OrderListener<OrderPtr>,
                             public liquibook::book::TradeListener<liquibook::book::OrderBook<OrderPtr>>,
                             public liquibook::book::DepthListener<Book> {
public:
  explicit ListenerBridge(std::vector<ListenerEvent>& sink) : sink_(sink) {}

  void on_accept(const OrderPtr& order) override;
  void on_trigger_stop(const OrderPtr& order) override;
  void on_reject(const OrderPtr& order, const char* reason) override;
  void on_fill(const OrderPtr& order,
               const OrderPtr& matched_order,
               liquibook::book::Quantity fill_qty,
               liquibook::book::Price fill_price) override;
  void on_cancel(const OrderPtr& order) override;
  void on_cancel_reject(const OrderPtr& order, const char* reason) override;
  void on_replace(const OrderPtr& order,
                  const int64_t& size_delta,
                  liquibook::book::Price new_price) override;
  void on_replace_reject(const OrderPtr& order, const char* reason) override;

  void on_trade(const liquibook::book::OrderBook<OrderPtr>* book,
                liquibook::book::Quantity qty,
                liquibook::book::Price price) override;
  void on_depth_change(const Book* book, const Book::DepthTracker* depth) override;

private:
  std::vector<ListenerEvent>& sink_;

  void push(const ListenerEvent& e);
};

}  // namespace sarvex::me
