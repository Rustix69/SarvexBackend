#include "mecore/server.h"

#include <grpcpp/grpcpp.h>

namespace sarvex::mecore {
namespace {

grpc::Status dispatch_status(const CommandMeta& result) {
  if (result.queue_full) return grpc::Status(grpc::StatusCode::RESOURCE_EXHAUSTED, "sequencer queue full");
  if (result.unknown) return grpc::Status(grpc::StatusCode::DEADLINE_EXCEEDED, "matching outcome is unknown");
  return grpc::Status::OK;
}

}  // namespace

grpc::Status MatchingEngineService::AddBook(
    grpc::ServerContext*, const sarvex::v1::AddBookRequest* request,
    google::protobuf::Empty*) {
  const auto result = engine_->add_book(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  if (!result.ok) {
    return grpc::Status(grpc::StatusCode::FAILED_PRECONDITION, result.reject_code);
  }
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::CloseBook(
    grpc::ServerContext*, const sarvex::v1::CloseBookRequest* request,
    sarvex::v1::CloseBookResponse* response) {
  const auto result = engine_->close_book(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  if (!result.closed) return grpc::Status(grpc::StatusCode::NOT_FOUND, result.reject_code);
  response->set_ticker(result.ticker);
  response->set_close_global_seq(result.close_global_seq);
  response->set_close_contract_seq(result.close_contract_seq);
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::SubmitOrder(
    grpc::ServerContext*, const sarvex::v1::MeSubmitOrderRequest* request,
    sarvex::v1::MeSubmitOrderResponse* response) {
  const auto result = engine_->submit_order(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  response->set_order_id(result.order_id);
  response->set_accepted(result.accepted);
  response->set_reject_code(result.reject_code);
  response->set_contract_seq(result.contract_seq);
  response->set_global_seq(result.global_seq);
  for (const auto& fill : result.fills) *response->add_fills() = fill;
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::CancelOrder(
    grpc::ServerContext*, const sarvex::v1::MeCancelOrderRequest* request,
    sarvex::v1::MeCancelOrderResponse* response) {
  const auto result = engine_->cancel_order(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  response->set_order_id(result.order_id);
  response->set_cancelled(result.cancelled);
  response->set_reject_code(result.reject_code);
  response->set_cancelled_qty(result.cancelled_qty);
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::AmendOrder(
    grpc::ServerContext*, const sarvex::v1::MeAmendOrderRequest* request,
    sarvex::v1::MeAmendOrderResponse* response) {
  const auto result = engine_->amend_order(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  response->set_order_id(result.order_id);
  response->set_amended(result.amended);
  response->set_reject_code(result.reject_code);
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::GetBookSnapshot(
    grpc::ServerContext*, const sarvex::v1::GetBookSnapshotRequest* request,
    sarvex::v1::BookSnapshot* response) {
  const auto result = engine_->snapshot(*request);
  if (const auto status = dispatch_status(result); !status.ok()) return status;
  if (!result.found) return grpc::Status(grpc::StatusCode::NOT_FOUND, "book not found");
  *response = result.snapshot;
  return grpc::Status::OK;
}

grpc::Status MatchingEngineService::StreamExecutions(
    grpc::ServerContext* context, const sarvex::v1::StreamExecutionsRequest* request,
    grpc::ServerWriter<sarvex::v1::ExecutionEvent>* writer) {
  const auto subscriber = engine_->subscribe(request->from_global_seq());
  while (!context->IsCancelled()) {
    sarvex::v1::ExecutionEvent event;
    const bool available = subscriber->pop(&event, [context] { return context->IsCancelled(); });
    if (!available) break;
    if (!writer->Write(event)) break;
  }
  engine_->unsubscribe(subscriber);
  return grpc::Status::OK;
}

}  // namespace sarvex::mecore
