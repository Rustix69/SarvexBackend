#pragma once

#include <book/order.h>
#include <book/types.h>

#include <cstdint>
#include <string>

namespace sarvex::me {

class SarvaOrder final : public liquibook::book::Order {
public:
  SarvaOrder(std::string order_id,
             std::string user_id,
             std::string hold_id,
             std::string ticker,
             liquibook::book::Quantity qty,
             liquibook::book::Price price,
             bool is_buy,
             bool immediate_or_cancel,
             bool all_or_none,
             uint32_t flags)
      : order_id_(std::move(order_id)),
        user_id_(std::move(user_id)),
        hold_id_(std::move(hold_id)),
        ticker_(std::move(ticker)),
        qty_(qty),
        price_(price),
        is_buy_(is_buy),
        ioc_(immediate_or_cancel),
        aon_(all_or_none),
        flags_(flags) {}

  bool is_buy() const override { return is_buy_; }
  liquibook::book::Price price() const override { return price_; }
  liquibook::book::Quantity order_qty() const override { return qty_; }
  bool all_or_none() const override { return aon_; }
  bool immediate_or_cancel() const override { return ioc_; }

  const std::string& order_id() const { return order_id_; }
  const std::string& user_id() const { return user_id_; }
  const std::string& hold_id() const { return hold_id_; }
  const std::string& ticker() const { return ticker_; }
  uint32_t flags() const { return flags_; }

private:
  std::string order_id_;
  std::string user_id_;
  std::string hold_id_;
  std::string ticker_;
  liquibook::book::Quantity qty_;
  liquibook::book::Price price_;
  bool is_buy_;
  bool ioc_;
  bool aon_;
  uint32_t flags_;
};

}  // namespace sarvex::me
