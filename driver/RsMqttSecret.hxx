// RsMqttSecret.hxx - passwords encrypted with _DriverSecurity.PublicKey.
//
// WinCC OA encrypts driver passwords with the RSA public key in
// _DriverSecurity.PublicKey; the driver decrypts them with the matching
// config/driver_private.key (searched in the project, the sub projects and
// the installation, like CTRL getPath(CONFIG_REL_PATH, ...)).
//
// Two formats exist, both stored as hex text in string elements:
//   Envelope  CTRL encrypt(plain, publicKey, crypt, "ENVELOPE_RSA_AES_256_CBC"),
//             usable in every manager (rsmqttEncryptPassword() in rsmqtt.ctl).
//             Starts with 01000000010000006400000064000000.
//   RawRsa    UI-only secureEncode() / drvsSecSetPassword() (driverSettings.ctl),
//             as used by the panels of the WinCC OA drivers: one RSA block,
//             i.e. 2 * key size hex digits (1024 for the default 4096 bit key).
// Anything else is a plain text password and is used as is.
#ifndef RSMQTT_SECRET_HXX
#define RSMQTT_SECRET_HXX

#include <string>

namespace RsMqttSecret
{
  enum class Format { Plain, Envelope, RawRsa };

  /// Format of a stored password (by its shape, nothing is decrypted).
  Format detect(const std::string &stored);

  /// Plain text of a stored password: decrypts Envelope / RawRsa values,
  /// returns plain values unchanged. On failure, `err` says why (never the secret).
  bool reveal(const std::string &stored, std::string &plain, std::string &err);

  /// Full path of the driver_private.key that is used (empty if none was found).
  std::string privateKeyPath();
}

#endif
