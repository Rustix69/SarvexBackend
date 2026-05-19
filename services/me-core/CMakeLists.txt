cmake_minimum_required(VERSION 3.20)
project(me_core CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(liquibook_headers INTERFACE)
target_include_directories(liquibook_headers INTERFACE
  ${CMAKE_SOURCE_DIR}/../../liquibook/src
)

add_executable(me-core src/main.cpp)
target_link_libraries(me-core PRIVATE liquibook_headers)
