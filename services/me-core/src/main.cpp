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
  auto submit = engine.submit_order(sarvex::me::SubmitOrderRequestView{
      .order_id = "demo-order-1",
      .user_id = "u_demo",
      .hold_id = "hold_demo",
      .ticker = "RBI-JUN26-CUT25",
      .price_ticks = 50,
      .qty = 10,
      .is_buy = true,
  });

  std::cout << "Sarvex me-core Milestone 7 sequencer running";
  if (snap.has_value()) {
    std::cout << " | ticker=" << snap->ticker
              << " bids=" << snap->bids.size()
              << " asks=" << snap->asks.size();
  }
  std::cout << " | submit.accepted=" << (submit.accepted ? "true" : "false")
            << " seq=" << submit.global_seq << "/" << submit.contract_seq;
  std::cout << std::endl;

  while (true) {
    std::this_thread::sleep_for(std::chrono::seconds(30));
  }
}
