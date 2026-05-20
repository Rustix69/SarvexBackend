#include "mecore/listener_bridge.h"

namespace sarvex::me {

void ListenerBridge::push(const ListenerEvent& e) { sink_.push_back(e); }

void ListenerBridge::on_accept(const OrderPtr& order) {
  push(ListenerEvent{.kind = "accept", .order_id = order->order_id()});
}

void ListenerBridge::on_trigger_stop(const OrderPtr& order) {
  push(ListenerEvent{.kind = "trigger_stop", .order_id = order->order_id()});
}

void ListenerBridge::on_reject(const OrderPtr& order, const char* /*reason*/) {
  push(ListenerEvent{.kind = "reject", .order_id = order->order_id()});
}

void ListenerBridge::on_fill(const OrderPtr& order,
                             const OrderPtr& matched_order,
                             liquibook::book::Quantity fill_qty,
                             liquibook::book::Price fill_price) {
  push(ListenerEvent{.kind = "fill",
                     .order_id = order->order_id(),
                     .matched_order_id = matched_order ? matched_order->order_id() : std::string{},
                     .qty = fill_qty,
                     .price = fill_price});
}

void ListenerBridge::on_cancel(const OrderPtr& order) {
  push(ListenerEvent{.kind = "cancel", .order_id = order->order_id()});
}

void ListenerBridge::on_cancel_reject(const OrderPtr& order, const char* /*reason*/) {
  push(ListenerEvent{.kind = "cancel_reject", .order_id = order->order_id()});
}

void ListenerBridge::on_replace(const OrderPtr& order,
                                const int64_t& /*size_delta*/,
                                liquibook::book::Price /*new_price*/) {
  push(ListenerEvent{.kind = "replace", .order_id = order->order_id()});
}

void ListenerBridge::on_replace_reject(const OrderPtr& order, const char* /*reason*/) {
  push(ListenerEvent{.kind = "replace_reject", .order_id = order->order_id()});
}

void ListenerBridge::on_trade(const liquibook::book::OrderBook<OrderPtr>* /*book*/,
                              liquibook::book::Quantity qty,
                              liquibook::book::Price price) {
  push(ListenerEvent{.kind = "trade", .qty = qty, .price = price});
}

void ListenerBridge::on_depth_change(const Book* /*book*/, const Book::DepthTracker* /*depth*/) {
  push(ListenerEvent{.kind = "depth_change"});
}

}  // namespace sarvex::me
