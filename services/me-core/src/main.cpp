#include "mecore/server.h"

#include <grpcpp/grpcpp.h>

#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>

int main() {
  const char* configured = std::getenv("ME_CORE_LISTEN_ADDR");
  const std::string address = configured == nullptr ? "0.0.0.0:50054" : configured;
  auto engine = std::make_shared<sarvex::mecore::Engine>();
  sarvex::mecore::MatchingEngineService service(engine);
  grpc::ServerBuilder builder;
  builder.AddListeningPort(address, grpc::InsecureServerCredentials());
  builder.RegisterService(&service);
  std::unique_ptr<grpc::Server> server(builder.BuildAndStart());
  if (!server) {
    std::cerr << "failed to start me-core on " << address << '\n';
    return 1;
  }
  std::cout << "me-core listening on " << address << '\n';
  server->Wait();
  return 0;
}
