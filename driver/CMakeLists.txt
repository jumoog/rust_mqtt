cmake_minimum_required(VERSION 3.21.1)

# get API root and remove optional quotes
set(API_ROOT_ENV "$ENV{API_ROOT}" CACHE FILEPATH "directory of the WinCC_OA API installation")
string(REPLACE "\"" "" API_ROOT "${API_ROOT_ENV}")
if ( NOT EXISTS "${API_ROOT}/CMakeDefines.txt" )
  message(FATAL_ERROR "API_ROOT='${API_ROOT}' is not a WinCC OA API directory (set the API_ROOT environment variable)")
endif()

include(${API_ROOT}/CMakeDefines.txt)

project(RsMqttDrv)

set(TARGET WCCOArsmqtt)

# ----------------------------------------------------------------------------
# rsmqtt (Rust) - built with cargo, linked statically
#
# rsmqtt_wincc.lib (crates/rsmqtt-wincc) holds the driver logic that needs no
# WinCC OA API (address syntax, JSON, Sparkplug B, redundancy names) and the
# rsmqtt client ABI: one Rust library, as two would bring the Rust standard
# library into the executable twice.

set(RSMQTT_ROOT ${CMAKE_CURRENT_SOURCE_DIR}/.. CACHE PATH "root of the rsmqtt cargo workspace")
find_program(CARGO cargo HINTS "$ENV{USERPROFILE}/.cargo/bin" "$ENV{HOME}/.cargo/bin" REQUIRED)

if ( WIN32 )
  set(RSMQTT_LIB ${RSMQTT_ROOT}/target/release/rsmqtt_wincc.lib)
  # from: cargo rustc -p rsmqtt-wincc --release -- --print native-static-libs
  set(RSMQTT_SYSLIBS bcrypt advapi32 kernel32 ntdll userenv ws2_32 dbghelp synchronization crypt32 secur32 ncrypt)
else()
  set(RSMQTT_LIB ${RSMQTT_ROOT}/target/release/librsmqtt_wincc.a)
  set(RSMQTT_SYSLIBS pthread dl m)
endif()

add_custom_target(rsmqtt_cargo
  COMMAND ${CARGO} build --release -p rsmqtt-wincc
  WORKING_DIRECTORY ${RSMQTT_ROOT}
  BYPRODUCTS ${RSMQTT_LIB}
  COMMENT "Building rsmqtt-wincc and rsmqtt (Rust, cargo build --release)"
  USES_TERMINAL
)

add_library(rsmqtt STATIC IMPORTED)
set_target_properties(rsmqtt PROPERTIES
  IMPORTED_LOCATION ${RSMQTT_LIB}
  INTERFACE_INCLUDE_DIRECTORIES "${RSMQTT_ROOT}/crates/rsmqtt/include;${RSMQTT_ROOT}/crates/rsmqtt-wincc/include"
  INTERFACE_LINK_LIBRARIES "${RSMQTT_SYSLIBS}"
)

# ----------------------------------------------------------------------------

set(SOURCES
  RsMqttRedu.cxx
  RsMqttBrokerDp.cxx
  RsMqttConnections.cxx
  RsMqttDrv.cxx
  RsMqttHWMapper.cxx
  RsMqttHWService.cxx
  RsMqttMain.cxx
  RsMqttResources.cxx
  RsMqttSecret.cxx
  RsMqttService.cxx
  RsMqttTrans.cxx
)

# ----------------------------------------------------------------------------
# OpenSSL for DrvSecurity (libComDrv): decrypts passwords encrypted with
# _DriverSecurity.PublicKey. The API does not ship OpenSSL headers, and
# DrvSecurity.hxx needs only a type name from them, so by default the stand-in
# headers in openssl/include are used and the driver is linked against the
# libcrypto of the WinCC OA installation (an import library is generated on
# Windows). RSMQTT_SYSTEM_OPENSSL=ON uses find_package(OpenSSL) instead.

option(RSMQTT_SYSTEM_OPENSSL "use the OpenSSL found by find_package() for DrvSecurity" OFF)

if ( RSMQTT_SYSTEM_OPENSSL )
  find_package(OpenSSL 3 REQUIRED COMPONENTS Crypto)
  set(RSMQTT_CRYPTO OpenSSL::Crypto)
  set(RSMQTT_OPENSSL_INCLUDE "")
elseif ( WIN32 )
  set(RSMQTT_CRYPTO_IMPLIB ${CMAKE_CURRENT_BINARY_DIR}/libcrypto-3-x64.lib)
  add_custom_command(
    OUTPUT ${RSMQTT_CRYPTO_IMPLIB}
    COMMAND ${CMAKE_AR} /nologo /machine:x64 /def:${CMAKE_CURRENT_SOURCE_DIR}/openssl/libcrypto-3-x64.def
            /out:${RSMQTT_CRYPTO_IMPLIB}
    DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/openssl/libcrypto-3-x64.def
    COMMENT "Generating the import library of libcrypto-3-x64.dll"
  )
  add_custom_target(rsmqtt_crypto_implib DEPENDS ${RSMQTT_CRYPTO_IMPLIB})
  set(RSMQTT_CRYPTO ${RSMQTT_CRYPTO_IMPLIB})
  set(RSMQTT_OPENSSL_INCLUDE ${CMAKE_CURRENT_SOURCE_DIR}/openssl/include)
else()
  # the libcrypto.so.3 of the installation (or of the system)
  find_library(RSMQTT_CRYPTO NAMES libcrypto.so.3 crypto
               HINTS ${API_ROOT}/../bin ${API_ROOT}/../lib REQUIRED)
  set(RSMQTT_OPENSSL_INCLUDE ${CMAKE_CURRENT_SOURCE_DIR}/openssl/include)
endif()

if ( WIN32 )
  set(SOURCES ${SOURCES} VersInfo.rc)
endif()

add_driver(${TARGET} ${SOURCES})
target_link_libraries(${TARGET} rsmqtt ${RSMQTT_CRYPTO})
add_dependencies(${TARGET} rsmqtt_cargo)
if ( TARGET rsmqtt_crypto_implib )
  add_dependencies(${TARGET} rsmqtt_crypto_implib)
endif()
if ( RSMQTT_OPENSSL_INCLUDE )
  set_source_files_properties(RsMqttSecret.cxx PROPERTIES INCLUDE_DIRECTORIES ${RSMQTT_OPENSSL_INCLUDE})
endif()

if ( MSVC )
  # the API headers are not /W4 clean; keep our own code at /W3 and UTF-8
  target_compile_options(${TARGET} PRIVATE /utf-8)
endif()

install(TARGETS ${TARGET} RUNTIME DESTINATION bin)
