#include "RsMqttConnections.hxx"
#include "RsMqttBrokerDp.hxx"
#include "RsMqttRedu.hxx"
#include "RsMqttSecret.hxx"

#include <ErrHdl.hxx>
#include <PVSSMacros.hxx>
#include <Resources.hxx>

#include <cctype>
#include <filesystem>

// TLS transports, as in paho.mqtt.rust's is_secure_uri()
static bool anySecureUri(const std::string &uri, const std::vector<std::string> &more)
{
  auto secure = [](const std::string &u) {
    for (const char *scheme : {"ssl://", "tls://", "mqtts://", "wss://"})
      if ( u.rfind(scheme, 0) == 0 )
        return true;
    return false;
  };
  if ( secure(uri) )
    return true;
  for (const auto &u : more)
    if ( secure(u) )
      return true;
  return false;
}

static void logError(ErrClass::ErrPrio prio, const char *func, const std::string &text)
{
  ErrHdl::error(prio, ErrClass::ERR_CONTROL,
                prio == ErrClass::PRIO_INFO ? ErrClass::NOERR : ErrClass::UNEXPECTEDSTATE,
                "RsMqttConnections", func, CharString(text.c_str()));
}

static std::string normalizedOptionName(const std::string &name)
{
  std::string n;
  for (char ch : name)
    if ( ch != '_' && ch != '-' )
      n += (char) std::tolower((unsigned char) ch);
  return n;
}

bool RsMqttConnection::isSecretOption(const std::string &name)
{
  return normalizedOptionName(name) == "password";
}

static std::string trimmed(const std::string &s)
{
  // the same characters as Rust's str::trim() in rsmqtt (ASCII part)
  const char *ws = " \t\r\n\v\f";
  size_t b = s.find_first_not_of(ws), e = s.find_last_not_of(ws);
  return b == std::string::npos ? std::string() : s.substr(b, e - b + 1);
}

void RsMqttConnection::noteOption(const std::string &name, const std::string &value)
{
  std::string n = normalizedOptionName(name);
  std::string v;
  for (char ch : trimmed(value))
    v += (char) std::tolower((unsigned char) ch);
  if ( n == "automaticreconnect" )
    autoReconnect = (v == "1" || v == "true" || v == "yes" || v == "on");
  else if ( n == "clientid" )
    clientId = value;
  else if ( n == "persistence" )
    persistence = (v == "file" || v == "default" || v == "1") ? 1 : 0;  // same names as rsmqtt
  else if ( n == "persistencedir" )
  {
    persistenceDirSet = true;
    persistenceDir = trimmed(value);
  }
}

void RsMqttConnection::applyReduOption(const std::string &option, const std::string &value)
{
  if ( !rsmqtt::Options::isKnown(option) )
    throw rsmqtt::Error(RSMQTT_UNKNOWN_OPTION, "unknown option 'redu." + option + "'");
  if ( !RsMqttRedu::isHost2() )
    return;
  options.set(option, value);
  noteOption(option, value);
  if ( normalizedOptionName(option) == "clientid" )
    reduClientId = true;
}

rsmqtt::Options RsMqttConnection::clientOptions() const
{
  rsmqtt::Options o = options;
  // the redundant hosts must not share a client id
  std::string id = RsMqttRedu::clientIdForHost(clientId, RsMqttRedu::isHost2(), reduClientId);
  if ( id != clientId )
    o.set("clientId", id);

  // as in rsmqtt: persistenceDir alone means file, an explicit persistence= wins
  bool file = persistence == 1 || (persistence == -1 && persistenceDirSet);
  if ( !file )
    return o;

  std::filesystem::path dir = std::filesystem::u8path(persistenceDir);
  if ( dir.empty() || dir.is_relative() )
  {
    std::filesystem::path data = std::filesystem::u8path((const char *) Resources::getDataDir());
    dir = dir.empty() ? data / "rsmqtt" : data / dir;
  }
  std::error_code ec;
  std::filesystem::create_directories(dir, ec);  // paho reports a folder it cannot use
  o.set("persistenceDir", dir.lexically_normal().generic_u8string());
  return o;
}

//--------------------------------------------------------------------------------

RsMqttConnection::RsMqttConnection(const std::string &n)
  : name(n), options(defaultOptions(n)), clientId(defaultClientId(n)) {}

std::string RsMqttConnection::defaultClientId(const std::string &n)
{
  return "WCCOArsmqtt_" + std::to_string((int) Resources::getManNum()) + "_" + n;
}

rsmqtt::Options RsMqttConnection::defaultOptions(const std::string &n)
{
  // Defaults suitable for a driver; every one can be overridden.
  rsmqtt::Options o;
  o.set("clientId", defaultClientId(n))
   .set("automaticReconnect", true)
   .set("minRetryInterval", 1)
   .set("maxRetryInterval", 30)
   .set("sendWhileDisconnected", true)
   .set("allowDisconnectedSendAtAnyTime", true)
   .set("maxBufferedMessages", 10000)
   .set("deleteOldestMessages", true)
   .set("completionEvents", "errors");
  return o;
}

bool RsMqttConnection::applyBrokerConfig(const RsMqttBrokerConfig &cfg, std::string &err)
{
  try
  {
    if ( cfg.serverURI.empty() && cfg.serverURIs.empty() )
    {
      err = "Config.ServerURI is empty";
      return false;
    }

    rsmqtt::Options o = defaultOptions(name);
    autoReconnect = true;  // driver default, see defaultOptions()
    clientId = defaultClientId(name);
    reduClientId = false;
    persistence = -1;
    persistenceDirSet = false;
    persistenceDir.clear();
    o.set("serverURI", cfg.serverURI.empty() ? cfg.serverURIs.front() : cfg.serverURI);
    if ( !cfg.serverURIs.empty() )
    {
      std::string list;
      for (const auto &u : cfg.serverURIs)
        list += (list.empty() ? "" : " ") + u;
      o.set("serverURIs", list);
    }
    if ( !cfg.clientId.empty() )   { o.set("clientId", cfg.clientId); clientId = cfg.clientId; }
    if ( cfg.mqttVersion != 0 )    o.set("MQTTVersion", cfg.mqttVersion);
    if ( cfg.keepAlive > 0 )       o.set("keepAliveInterval", cfg.keepAlive);
    o.set("cleansession", cfg.cleanSession);
    if ( !cfg.username.empty() )   o.set("username", cfg.username);
    if ( !cfg.password.empty() )
    {
      std::string plain;
      if ( !RsMqttSecret::reveal(cfg.password, plain, err) )
      {
        err = "Config.Password: " + err;
        return false;
      }
      o.set("password", plain);
    }
    if ( !cfg.tlsTrustStore.empty() ) o.set("ssl.trustStore", cfg.tlsTrustStore);
    if ( !cfg.tlsKeyStore.empty() )   o.set("ssl.keyStore", cfg.tlsKeyStore);
    if ( !cfg.tlsPrivateKey.empty() ) o.set("ssl.privateKey", cfg.tlsPrivateKey);
    // Config.TlsVerify decides the host-name check both ways (the chain is
    // checked either way). Only for TLS URIs: ssl.* options attach TLS
    // settings to the connection. Config.Options "ssl.verify=..." still wins.
    bool secure = anySecureUri(cfg.serverURI, cfg.serverURIs);
    if ( secure )                     o.set("ssl.verify", cfg.tlsVerify);
    if ( !cfg.willTopic.empty() )
    {
      o.set("will.topic", cfg.willTopic)
       .set("will.payload", cfg.willPayload)
       .set("will.qos", cfg.willQoS)
       .set("will.retained", cfg.willRetain);
    }
    // redu.<option> entries last: on the second redundant host they win
    std::vector<std::pair<std::string, std::string>> redu;
    for (const auto &opt : cfg.options)
    {
      if ( opt.empty() ) continue;
      size_t eq = opt.find('=');
      if ( eq == std::string::npos )
      {
        err = "Config.Options entry '" + opt + "' is not 'name=value'";
        return false;
      }
      std::string optName = opt.substr(0, eq), value = opt.substr(eq + 1), reduName;
      if ( RsMqttRedu::splitReduOption(trimmed(optName), reduName) )
      {
        redu.emplace_back(trimmed(reduName), value);
        continue;
      }
      if ( isSecretOption(optName) && !RsMqttSecret::reveal(value, value, err) )
      {
        err = "Config.Options '" + optName + "': " + err;
        return false;
      }
      o.set(optName, value);
      noteOption(optName, value);
    }
    // applyReduOption works on `options`; the previous ones come back on failure
    rsmqtt::Options previous = options;
    options = o;
    for (auto &r : redu)
    {
      if ( RsMqttRedu::isHost2() && isSecretOption(r.first) && !RsMqttSecret::reveal(r.second, r.second, err) )
      {
        options = previous;
        err = "Config.Options 'redu." + r.first + "': " + err;
        return false;
      }
      try
      {
        applyReduOption(r.first, r.second);
      }
      catch (const rsmqtt::Error &)
      {
        options = previous;
        throw;
      }
    }
    o = options;
    options = previous;

    if ( cfg.subscribeQoS < 0 || cfg.subscribeQoS > 2 || cfg.publishQoS < 0 || cfg.publishQoS > 2 )
    {
      err = "Config.SubscribeQoS / Config.PublishQoS must be 0, 1 or 2";
      return false;
    }

    options = o;
    subscribeQoS = cfg.subscribeQoS;
    publishQoS = cfg.publishQoS;
    publishRetain = cfg.publishRetain;
    return true;
  }
  catch (const rsmqtt::Error &e)
  {
    err = e.what();
    return false;
  }
}

bool RsMqttConnection::isActive() const
{
  switch ( source )
  {
    case Source::ConfigFile:  return true;
    case Source::Datapoint:   return listed && brokerDp && brokerDp->isReady() && brokerDp->config().active;
    default:                  return false;
  }
}

int RsMqttConnection::connState() const
{
  // FAILURE needs action (bad configuration); a lost or refused connection
  // that is being retried is DISCONNECTED, with the reason in LastError.
  if ( !isActive() )             return RSMQTT_CS_DISABLED;
  if ( stopped )                 return RSMQTT_CS_DISCONNECTED;  // driver stop, not a fault
  if ( configError || !client )  return RSMQTT_CS_FAILURE;
  if ( connected )               return RSMQTT_CS_CONNECTED;
  if ( connecting )              return RSMQTT_CS_CONNECTING;
  return RSMQTT_CS_DISCONNECTED;
}

void RsMqttConnection::subscribe(const std::vector<std::string> &filters)
{
  if ( !client || !connected || filters.empty() )
    return;

  const size_t batch = 64;
  for (size_t i = 0; i < filters.size(); i += batch)
  {
    std::vector<std::string> f;
    std::vector<int> qos, flags;
    for (size_t j = i; j < filters.size() && j < i + batch; j++)
    {
      auto it = subscriptions.find(filters[j]);
      if ( it == subscriptions.end() ) continue;
      f.push_back(it->first);
      qos.push_back(it->second.qos);
      flags.push_back(it->second.flags);
    }
    if ( f.empty() ) continue;
    try
    {
      rsmqtt::Token tok = client->subscribe(f, qos, flags);
      subscribeTokens[tok] = f;
      DEBUG_DRV_USR1("rsmqtt [" << name.c_str() << "] subscribing " << f.size() << " filter(s), token " << (long) tok);
    }
    catch (const rsmqtt::Error &e)
    {
      logError(ErrClass::PRIO_WARNING, "subscribe", "[" + name + "] subscribe failed: " + e.what());
    }
  }
}

void RsMqttConnection::subscribeAll()
{
  std::vector<std::string> f;
  for (const auto &s : subscriptions)
    f.push_back(s.first);
  subscribe(f);
}

void RsMqttConnection::unsubscribe(const std::string &filter)
{
  if ( !client || !connected )
    return;
  try
  {
    client->unsubscribe(filter);
  }
  catch (const rsmqtt::Error &e)
  {
    logError(ErrClass::PRIO_WARNING, "unsubscribe", "[" + name + "] unsubscribe '" + filter + "' failed: " + e.what());
  }
}

//--------------------------------------------------------------------------------

RsMqttConnections &RsMqttConnections::instance()
{
  static RsMqttConnections self;
  return self;
}

RsMqttConnection *RsMqttConnections::add(const std::string &name)
{
  if ( RsMqttConnection *c = find(name) )
    return c;
  list.push_back(std::make_unique<RsMqttConnection>(name));
  return list.back().get();
}

RsMqttConnection *RsMqttConnections::find(const std::string &name)
{
  for (auto &c : list)
    if ( c->name == name )
      return c.get();
  return nullptr;
}

void RsMqttConnections::setBrokerList(const std::vector<std::string> &names)
{
  std::set<std::string> wanted;
  for (const auto &raw : names)
  {
    std::string n = raw;
    // tolerate the DP name instead of the broker name
    if ( n.rfind("_RsMqtt_", 0) == 0 ) n = n.substr(8);
    if ( n.empty() ) continue;
    if ( n.find_first_of(":;. ") != std::string::npos )
    {
      logError(ErrClass::PRIO_WARNING, "setBrokerList", "invalid broker name '" + n + "'");
      continue;
    }
    // in a redundant system "<x>_2" names the second host's twin of "<x>"
    if ( RsMqttRedu::isRedundant() && RsMqttRedu::endsWithSuffix(n) )
    {
      logError(ErrClass::PRIO_WARNING, "setBrokerList",
               "broker name '" + n + "' ends with \"_2\", which is reserved for the second redundant host; ignored");
      continue;
    }
    wanted.insert(n);
  }

  for (const auto &n : wanted)
  {
    RsMqttConnection *c = add(n);
    if ( c->source == RsMqttConnection::Source::ConfigFile )
    {
      logError(ErrClass::PRIO_WARNING, "setBrokerList",
               "broker '" + n + "' is configured in the config file and as datapoint; the datapoint is ignored");
      continue;
    }
    // A broker whose DP was deleted may have been created again: its old
    // DrvIntDp still holds the deleted DP's ids, so it gets a new one. The
    // old object stays in brokerDps (DrvIntDp objects must live until exit).
    if ( !c->brokerDp || c->brokerDp->isDeleted() )
    {
      bool replaced = (c->brokerDp != nullptr);
      brokerDps.push_back(std::make_unique<RsMqttBrokerIntDp>(n));
      c->brokerDp = brokerDps.back().get();
      if ( replaced )
      {
        logError(ErrClass::PRIO_INFO, "setBrokerList", "broker '" + n + "': datapoint connected again");
        c->dirty = true;
        c->dirtySince = std::chrono::steady_clock::now();
      }
    }
    c->source = RsMqttConnection::Source::Datapoint;
    if ( !c->listed )
    {
      c->listed = true;
      c->dirty = true;
      c->dirtySince = std::chrono::steady_clock::now();
    }
  }

  for (auto &c : list)
  {
    if ( c->source == RsMqttConnection::Source::Datapoint && c->listed && !wanted.count(c->name) )
    {
      c->listed = false;
      c->dirty = true;
      c->dirtySince = std::chrono::steady_clock::now();
    }
  }
}

const RsMqttAddress *RsMqttConnections::parsed(const std::string &fullAddress) const
{
  auto it = addresses.find(fullAddress);
  return it == addresses.end() ? nullptr : &it->second.first;
}

bool RsMqttConnections::addAddress(const std::string &fullAddress, const RsMqttAddress &addr, bool input)
{
  RsMqttConnection *conn = add(addr.connection);  // placeholder if not configured (yet)

  auto &entry = addresses[fullAddress];
  bool isNew = (entry.second++ == 0);
  entry.first = addr;

  if ( addr.isState )
  {
    conn->stateAddresses[fullAddress]++;
    conn->statePushNeeded = true;  // HWService writes the current state
    return isNew;
  }
  if ( !input )
    return isNew;

  int qos = addr.qos >= 0 ? addr.qos : conn->subscribeQoS;
  bool newFilter = (conn->subscriptions.find(addr.topic) == conn->subscriptions.end());
  RsMqttConnection::Subscription &sub = conn->subscriptions[addr.topic];
  bool changed = newFilter || qos > sub.qos || (addr.subFlags & ~sub.flags);
  if ( newFilter || qos > sub.qos )
    sub.qos = qos;
  sub.flags |= addr.subFlags;
  sub.matching = addr.needsMatching();
  sub.addresses[fullAddress]++;
  sub.planDirty = true;
  if ( sub.matching )
    conn->matchingFilters.insert(addr.topic);

  // (Re-)subscribe right away if connected; otherwise it happens on connect.
  if ( changed )
    conn->subscribe({addr.topic});

  return isNew;
}

bool RsMqttConnections::removeAddress(const std::string &fullAddress, const RsMqttAddress &addr, bool input)
{
  auto it = addresses.find(fullAddress);
  if ( it == addresses.end() )
    return false;
  bool last = (--it->second.second == 0);
  if ( last )
    addresses.erase(it);

  RsMqttConnection *conn = find(addr.connection);
  if ( !conn )
    return last;

  if ( addr.isState )
  {
    auto s = conn->stateAddresses.find(fullAddress);
    if ( s != conn->stateAddresses.end() && --s->second == 0 )
      conn->stateAddresses.erase(s);
    return last;
  }
  if ( !input )
    return last;

  auto s = conn->subscriptions.find(addr.topic);
  if ( s == conn->subscriptions.end() )
    return last;
  auto a = s->second.addresses.find(fullAddress);
  if ( a != s->second.addresses.end() && --a->second == 0 )
    s->second.addresses.erase(a);
  s->second.planDirty = true;
  if ( s->second.addresses.empty() )
  {
    conn->matchingFilters.erase(addr.topic);
    conn->subscriptions.erase(s);
    conn->unsubscribe(addr.topic);
  }
  return last;
}
