#include <chrono>
#include <iostream>
#include <thread>

#include "mecore/me_core_engine.h"

int main() {
  sarvex::me::MeCoreEngine engine;
  std::string reject_reason;
  const bool ok = engine.add_book("RBI-JUN26-CUT25",
                                  sarvex::me::CONTRACT_KIND_BINARY,
                                  1,
                                  1,
                                  99,
                                  &reject_reason);
  if (!ok) {
    std::cerr << "Failed to add demo book: " << reject_reason << std::endl;
    return 1;
  }

  auto snap = engine.get_book_snapshot("RBI-JUN26-CUT25", 10);
  std::cout << "Sarvex me-core Milestone 6 scaffold running";
  if (snap.has_value()) {
    std::cout << " | ticker=" << snap->ticker
              << " bids=" << snap->bids.size()
              << " asks=" << snap->asks.size();
  }
  std::cout << std::endl;

  while (true) {
    std::this_thread::sleep_for(std::chrono::seconds(30));
  }
}
