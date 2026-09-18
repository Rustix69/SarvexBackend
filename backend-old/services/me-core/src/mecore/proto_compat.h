#pragma once

#if defined(__has_include)
#if __has_include("google/protobuf/runtime_version.h") && __has_include("sarvex/v1/common.pb.h")
#define SARVEX_ME_PROTO_AVAILABLE 1
#include "sarvex/v1/common.pb.h"
namespace sarvex::me {
using ContractKind = sarvex::v1::ContractKind;
constexpr ContractKind CONTRACT_KIND_UNSPECIFIED = sarvex::v1::CONTRACT_KIND_UNSPECIFIED;
constexpr ContractKind CONTRACT_KIND_BINARY = sarvex::v1::CONTRACT_KIND_BINARY;
constexpr ContractKind CONTRACT_KIND_SCALAR = sarvex::v1::CONTRACT_KIND_SCALAR;
}  // namespace sarvex::me
#else
#define SARVEX_ME_PROTO_AVAILABLE 0
namespace sarvex::me {
enum ContractKind {
  CONTRACT_KIND_UNSPECIFIED = 0,
  CONTRACT_KIND_BINARY = 1,
  CONTRACT_KIND_SCALAR = 2,
};
}  // namespace sarvex::me
#endif
#else
#define SARVEX_ME_PROTO_AVAILABLE 0
namespace sarvex::me {
enum ContractKind {
  CONTRACT_KIND_UNSPECIFIED = 0,
  CONTRACT_KIND_BINARY = 1,
  CONTRACT_KIND_SCALAR = 2,
};
}  // namespace sarvex::me
#endif
