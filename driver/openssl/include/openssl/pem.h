/* Stand-in for the OpenSSL header included by the WinCC OA API header
   ComDrv/DrvSecurity.hxx, which only needs the RSA type name for private
   method declarations. The API does not ship OpenSSL headers; the code is
   linked against the OpenSSL 3 libcrypto of the WinCC OA installation
   (see CMakeLists.txt). RSMQTT_SYSTEM_OPENSSL=ON uses real ones. */
#pragma once
typedef struct rsa_st RSA;
