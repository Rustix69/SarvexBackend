#include <chrono>
#include <iostream>
#include <thread>

int main() {
  std::cout << "Sarvex me-core Milestone 3 skeleton running" << std::endl;
  while (true) {
    std::this_thread::sleep_for(std::chrono::seconds(30));
  }
}
