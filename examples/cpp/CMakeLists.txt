# Standalone C++ examples/tests of rsmqtt (no WinCC OA needed):
#   cmake -S examples/cpp -B examples/cpp/build && cmake --build examples/cpp/build --config Release
cmake_minimum_required(VERSION 3.21)
project(rsmqtt_cpp_examples CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")   # /MD, same as Rust's MSVC target

set(RSMQTT_ROOT ${CMAKE_CURRENT_SOURCE_DIR}/../..)
find_program(CARGO cargo HINTS "$ENV{USERPROFILE}/.cargo/bin" "$ENV{HOME}/.cargo/bin" REQUIRED)

if ( WIN32 )
  set(RSMQTT_LIB ${RSMQTT_ROOT}/target/release/rsmqtt.lib)
  set(RSMQTT_SYSLIBS bcrypt advapi32 kernel32 ntdll userenv ws2_32 dbghelp synchronization crypt32 secur32 ncrypt)
else()
  set(RSMQTT_LIB ${RSMQTT_ROOT}/target/release/librsmqtt.a)
  set(RSMQTT_SYSLIBS pthread dl m)
endif()

add_custom_target(rsmqtt_cargo
  COMMAND ${CARGO} build --release -p rsmqtt
  WORKING_DIRECTORY ${RSMQTT_ROOT}
  BYPRODUCTS ${RSMQTT_LIB}
  USES_TERMINAL)

add_library(rsmqtt STATIC IMPORTED)
set_target_properties(rsmqtt PROPERTIES
  IMPORTED_LOCATION ${RSMQTT_LIB}
  INTERFACE_INCLUDE_DIRECTORIES ${RSMQTT_ROOT}/crates/rsmqtt/include
  INTERFACE_LINK_LIBRARIES "${RSMQTT_SYSLIBS}")

add_executable(pubsub pubsub.cpp)
target_link_libraries(pubsub rsmqtt)
add_dependencies(pubsub rsmqtt_cargo)

# unit test of the driver's address parser / topic matcher
add_executable(address_test address_test.cpp ${RSMQTT_ROOT}/driver/RsMqttAddress.cxx)
target_include_directories(address_test PRIVATE ${RSMQTT_ROOT}/driver)
