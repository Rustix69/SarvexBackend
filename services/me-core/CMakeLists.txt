cmake_minimum_required(VERSION 3.20)
project(sarvex_me_core LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

get_filename_component(SARVEX_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(PROTO_ROOT "${SARVEX_REPO_ROOT}/backend-old/proto")
set(LIQUIBOOK_ROOT "${SARVEX_REPO_ROOT}/backend-old/liquibook/src")
set(GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")
file(MAKE_DIRECTORY "${GENERATED_DIR}")

find_package(Protobuf REQUIRED)
find_package(gRPC CONFIG REQUIRED)
find_package(Threads REQUIRED)

set(PROTO_FILES
  "${PROTO_ROOT}/sarvex/v1/common.proto"
  "${PROTO_ROOT}/sarvex/v1/match.proto"
)
set(GENERATED_FILES
  "${GENERATED_DIR}/sarvex/v1/common.pb.cc"
  "${GENERATED_DIR}/sarvex/v1/common.pb.h"
  "${GENERATED_DIR}/sarvex/v1/match.pb.cc"
  "${GENERATED_DIR}/sarvex/v1/match.pb.h"
  "${GENERATED_DIR}/sarvex/v1/match.grpc.pb.cc"
  "${GENERATED_DIR}/sarvex/v1/match.grpc.pb.h"
)

add_custom_command(
  OUTPUT ${GENERATED_FILES}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${GENERATED_DIR}/sarvex/v1"
  COMMAND ${Protobuf_PROTOC_EXECUTABLE}
    --proto_path=${PROTO_ROOT}
    --cpp_out=${GENERATED_DIR}
    --grpc_out=${GENERATED_DIR}
    --plugin=protoc-gen-grpc=$<TARGET_FILE:gRPC::grpc_cpp_plugin>
    ${PROTO_FILES}
  DEPENDS ${PROTO_FILES} gRPC::grpc_cpp_plugin
  VERBATIM
)

add_executable(me-core
  src/main.cpp
  src/server.cpp
  src/engine.cpp
  ${GENERATED_FILES}
)
target_include_directories(me-core PRIVATE
  include
  "${GENERATED_DIR}"
  "${LIQUIBOOK_ROOT}"
)
target_link_libraries(me-core PRIVATE
  gRPC::grpc++
  protobuf::libprotobuf
  Threads::Threads
)
target_compile_options(me-core PRIVATE -Wall -Wextra -Wpedantic)

install(TARGETS me-core RUNTIME DESTINATION bin)
