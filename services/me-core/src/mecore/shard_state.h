#pragma once

#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

#include "mecore/book_state.h"
#include "mecore/listener_bridge.h"

namespace sarvex::me {

struct ShardState {
  std::unordered_map<std::string, std::unique_ptr<BookState>> books_by_ticker;
  std::unordered_map<std::string, std::unique_ptr<SarvaOrder>> orders_by_id;

  uint64_t global_seq{0};
  std::vector<ListenerEvent> listener_events;
  std::unique_ptr<ListenerBridge> listener;

  ShardState() : listener(std::make_unique<ListenerBridge>(listener_events)) {}
};

}  // namespace sarvex::me
