#pragma once

#include <book/depth_order_book.h>

#include <memory>
#include <string>

#include "mecore/sarva_order.h"

namespace sarvex::me {

using OrderPtr = SarvaOrder*;
using Book = liquibook::book::DepthOrderBook<OrderPtr>;

struct BookState {
  explicit BookState(std::string t) : ticker(std::move(t)), book(std::make_unique<Book>(ticker)) {}

  std::string ticker;
  std::unique_ptr<Book> book;
  uint64_t contract_seq{0};
  bool closed{false};
};

}  // namespace sarvex::me
