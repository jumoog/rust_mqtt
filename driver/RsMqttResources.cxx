#include "RsMqttResources.hxx"
#include "RsMqttConnections.hxx"
#include "RsMqttSecret.hxx"

#include <ErrHdl.hxx>

#include <chrono>
#include <string>
#include <vector>

namespace
{
  // config file options that may hold an encrypted value
  struct PendingSecret
  {
    RsMqttConnection *connection;
    std::string name, value;
  };
  std::vector<PendingSecret> pendingSecrets;
}

unsigned          RsMqttResources::maxPayload = 65536;
int               RsMqttResources::traceLevel = 0;
RsMqttConnection *RsMqttResources::current = nullptr;
CharString        RsMqttResources::driverDp;

static void cfgError(const CharString &text)
{
  ErrHdl::error(ErrClass::PRIO_SEVERE, ErrClass::ERR_PARAM, ErrClass::UNEXPECTEDSTATE,
                "RsMqttResources", "readSection", text);
}

//-------------------------------------------------------------------------------

void RsMqttResources::init(int &argc, char *argv[])
{
  begin(argc, argv);
  while ( readSection() || generalSection() )
    ;
  end(argc, argv);

  for (auto &p : pendingSecrets)
  {
    std::string plain, err;
    if ( !RsMqttSecret::reveal(p.value, plain, err) )
    {
      // the connection is still made, without this option; the broker reports the rest
      cfgError(CharString("connection '") + p.connection->name.c_str() + "', " + p.name.c_str() + ": " + err.c_str());
      continue;
    }
    try
    {
      p.connection->options.set(p.name, plain);
    }
    catch (const rsmqtt::Error &e)
    {
      cfgError(CharString("connection '") + p.connection->name.c_str() + "': " + e.what());
    }
  }
  pendingSecrets.clear();

  if ( driverDp.len() == 0 )
    driverDp = CharString("_RsMqttDriver_") + CharString((int) getManNum());
}

//-------------------------------------------------------------------------------

PVSSboolean RsMqttResources::readSection()
{
  if ( !isSection("rsmqtt") )
    return PVSS_FALSE;

  // Each [rsmqtt] / [rsmqtt_<num>] section starts without a current connection.
  current = nullptr;

  getNextEntry();

  while ( (getCfgState() != CFG_SECT_START) && (getCfgState() != CFG_EOF) )
  {
    const CharString &kw = getKeyWord();

    if ( kw.icmp("connection") == 0 )
    {
      CharString name;
      getCfgStream() >> name;
      if ( name.len() == 0 || strchr((const char *) name, ':') )
      {
        cfgError(CharString("invalid connection name '") + name + "'");
        current = nullptr;
      }
      else
      {
        current = RsMqttConnections::instance().add((const char *) name);
        current->source = RsMqttConnection::Source::ConfigFile;
      }
    }
    else if ( kw.icmp("driverDp") == 0 )
      getCfgStream() >> driverDp;
    else if ( kw.icmp("maxPayload") == 0 )
    {
      getCfgStream() >> maxPayload;
      if ( maxPayload < 16 ) maxPayload = 16;
      if ( maxPayload > 256u * 1024u * 1024u ) maxPayload = 256u * 1024u * 1024u;
    }
    else if ( kw.icmp("traceLevel") == 0 )
      getCfgStream() >> traceLevel;
    else if ( current && kw.icmp("subscribeQoS") == 0 )
      getCfgStream() >> current->subscribeQoS;
    else if ( current && kw.icmp("publishQoS") == 0 )
      getCfgStream() >> current->publishQoS;
    else if ( current && kw.icmp("publishRetain") == 0 )
    {
      int b = 0;
      getCfgStream() >> b;
      current->publishRetain = (b != 0);
    }
    else if ( current && kw.icmp("reconnectInterval") == 0 )
    {
      double s = 10;
      getCfgStream() >> s;
      if ( s < 0.1 ) s = 0.1;
      current->reconnectInterval = std::chrono::milliseconds((long long) (s * 1000));
    }
    else if ( rsmqtt::Options::isKnown((const char *) kw) )
    {
      CharString value;
      getCfgStream() >> value;
      if ( !current )
        cfgError(CharString("'") + kw + "' must follow a 'connection = \"<name>\"' entry");
      else if ( RsMqttConnection::isSecretOption((const char *) kw) )
      {
        // may be encrypted: decrypted in init() once all paths are known
        pendingSecrets.push_back({current, std::string((const char *) kw), std::string((const char *) value)});
      }
      else
      {
        try
        {
          current->options.set((const char *) kw, (const char *) value);
          current->noteOption((const char *) kw, (const char *) value);
        }
        catch (const rsmqtt::Error &e)
        {
          cfgError(CharString("connection '") + current->name.c_str() + "': " + e.what());
        }
      }
    }
    else if ( !commonKeyWord() )
    {
      unknownKeyWordError();
    }

    getNextEntry();
  }

  return getCfgState() != CFG_EOF;
}
