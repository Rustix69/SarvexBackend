cmake_minimum_required(VERSION 3.20)
project(me_core CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(liquibook_headers INTERFACE)
target_include_directories(liquibook_headers INTERFACE
  ${CMAKE_SOURCE_DIR}/../../liquibook/src
)

add_library(me_core_proto_headers INTERFACE)
target_include_directories(me_core_proto_headers INTERFACE
  ${CMAKE_SOURCE_DIR}/gen
)

add_library(me_core_lib
  src/mecore/listener_bridge.cpp
  src/mecore/me_core_engine.cpp
)
target_include_directories(me_core_lib PUBLIC
  ${CMAKE_SOURCE_DIR}/src
)
target_link_libraries(me_core_lib
  PUBLIC liquibook_headers
  PUBLIC me_core_proto_headers
)

add_executable(me-core src/main.cpp)
target_link_libraries(me-core PRIVATE me_core_lib)
