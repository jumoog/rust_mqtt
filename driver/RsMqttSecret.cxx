#include "RsMqttSecret.hxx"

#include <Blob.hxx>
#include <DrvSecurity.hxx>  // needs OpenSSL headers, see CMakeLists.txt
#include <FileSys.hxx>
#include <Resources.hxx>
#include <Cryptography/CipherUtility.hxx>
#include <Cryptography/SecureDataBuffer.hxx>

#include <cctype>
#include <fstream>
#include <memory>
#include <sstream>

namespace
{
  const char ENVELOPE_PREFIX[] = "01000000010000006400000064000000";
  const char KEY_FILE[] = "driver_private.key";

  bool isHex(const std::string &s)
  {
    if ( s.empty() || (s.size() % 2) != 0 )
      return false;
    for (char ch : s)
      if ( !std::isxdigit((unsigned char) ch) )
        return false;
    return true;
  }

  bool startsWithNoCase(const std::string &s, const char *prefix)
  {
    for (size_t i = 0; prefix[i]; i++)
      if ( i >= s.size() || std::toupper((unsigned char) s[i]) != prefix[i] )
        return false;
    return true;
  }

  int hexValue(char ch)
  {
    if ( ch >= '0' && ch <= '9' ) return ch - '0';
    return std::toupper((unsigned char) ch) - 'A' + 10;
  }

  void wipe(std::string &s)
  {
    volatile char *p = s.empty() ? nullptr : &s[0];
    for (size_t i = 0; i < s.size(); i++)
      p[i] = 0;
    s.clear();
  }

  bool readFile(const std::string &path, std::string &content)
  {
    std::ifstream f(path, std::ios::binary);
    if ( !f )
      return false;
    std::stringstream ss;
    ss << f.rdbuf();
    content = ss.str();
    return !content.empty();
  }

  bool revealEnvelope(const std::string &stored, const std::string &keyPath, std::string &plain, std::string &err)
  {
    using namespace Cryptography;

    std::string pem;
    if ( !readFile(keyPath, pem) )
    {
      err = "cannot read " + keyPath;
      return false;
    }
    auto passphrase = std::make_shared<SecureDataBuffer>(pem);
    wipe(pem);

    // CipherUtility accepts the hex text and decodes it itself
    std::shared_ptr<ISecureDataBuffer> crypt = std::make_shared<SecureDataBuffer>(stored), out;
    ErrorCode ec = CipherUtility::decrypt(crypt, passphrase, out);
    if ( !ec.isSuccess() || !out )
    {
      err = "decryption failed (envelope format; does " + keyPath + " match _DriverSecurity.PublicKey?)";
      return false;
    }
    plain.assign((const char *) out->data(), out->size());
    return true;
  }

  bool revealRawRsa(const std::string &stored, const std::string &keyPath, std::string &plain, std::string &err)
  {
    std::string raw;
    raw.reserve(stored.size() / 2);
    for (size_t i = 0; i + 1 < stored.size(); i += 2)
      raw += (char) ((hexValue(stored[i]) << 4) | hexValue(stored[i + 1]));

    Blob crypt, out;
    crypt.copyCharData(raw.data(), raw.size());
    DrvSecurity sec;
    if ( !sec.decrypt(crypt, CharString(keyPath.c_str()), out, false) )
    {
      err = "decryption failed (RSA format; does " + keyPath + " match _DriverSecurity.PublicKey?)";
      return false;
    }
    plain.assign((const char *) out.getData(), out.getLen());
    return true;
  }
}

namespace RsMqttSecret
{

Format detect(const std::string &stored)
{
  if ( !isHex(stored) )
    return Format::Plain;
  if ( startsWithNoCase(stored, ENVELOPE_PREFIX) )
    return Format::Envelope;
  // one RSA block of a 1024 .. 8192 bit key
  switch ( stored.size() / 2 )
  {
    case 128: case 256: case 384: case 512: case 768: case 1024:
      return Format::RawRsa;
  }
  return Format::Plain;
}

std::string privateKeyPath()
{
  // project, sub projects, installation
  for (int level = 1; level <= Resources::getSearchPathLen(); level++)
  {
    CharString path = Resources::getConfigDir(level) + KEY_FILE;
    if ( FileSys::fileExists(path) )
      return std::string((const char *) path, path.len());
  }
  return std::string();
}

bool reveal(const std::string &stored, std::string &plain, std::string &err)
{
  Format fmt = detect(stored);
  if ( fmt == Format::Plain )
  {
    plain = stored;
    return true;
  }

  std::string keyPath = privateKeyPath();
  if ( keyPath.empty() )
  {
    err = std::string("encrypted password, but no config/") + KEY_FILE + " found in the project or the installation";
    return false;
  }
  return fmt == Format::Envelope ? revealEnvelope(stored, keyPath, plain, err)
                                 : revealRawRsa(stored, keyPath, plain, err);
}

}
