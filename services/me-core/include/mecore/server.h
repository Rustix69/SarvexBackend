#pragma once

#include "mecore/engine.h"
#include "sarvex/v1/match.grpc.pb.h"

#include <memory>

namespace sarvex::mecore {

class MatchingEngineService final : public sarvex::v1::MatchingEngine::Service {
 public:
  explicit MatchingEngineService(std::shared_ptr<Engine> engine)
      : engine_(std::move(engine)) {}

  grpc::Status AddBook(grpc::ServerContext* context,
                       const sarvex::v1::AddBookRequest* request,
                       google::protobuf::Empty* response) override;
  grpc::Status CloseBook(grpc::ServerContext* context,
                         const sarvex::v1::CloseBookRequest* request,
                         sarvex::v1::CloseBookResponse* response) override;
  grpc::Status SubmitOrder(grpc::ServerContext* context,
                           const sarvex::v1::MeSubmitOrderRequest* request,
                           sarvex::v1::MeSubmitOrderResponse* response) override;
  grpc::Status CancelOrder(grpc::ServerContext* context,
                           const sarvex::v1::MeCancelOrderRequest* request,
                           sarvex::v1::MeCancelOrderResponse* response) override;
  grpc::Status AmendOrder(grpc::ServerContext* context,
                          const sarvex::v1::MeAmendOrderRequest* request,
                          sarvex::v1::MeAmendOrderResponse* response) override;
  grpc::Status GetBookSnapshot(grpc::ServerContext* context,
                               const sarvex::v1::GetBookSnapshotRequest* request,
                               sarvex::v1::BookSnapshot* response) override;
  grpc::Status StreamExecutions(
      grpc::ServerContext* context,
      const sarvex::v1::StreamExecutionsRequest* request,
      grpc::ServerWriter<sarvex::v1::ExecutionEvent>* writer) override;

 private:
  std::shared_ptr<Engine> engine_;
};

}  // namespace sarvex::mecore
