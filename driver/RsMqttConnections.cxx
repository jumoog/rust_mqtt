#include "RsMqttConnections.hxx"
#include "RsMqttBrokerDp.hxx"
#include "RsMqttRedu.hxx"
#include "RsMqttSecret.hxx"

#include <ErrHdl.hxx>
#include <PVSSMacros.hxx>
#include <Resources.hxx>

#include <chrono>
#include <set>

static void logError(ErrClass::ErrPrio prio, const char *func, const std::string &text)
{
  ErrHdl::error(prio, ErrClass::ERR_CONTROL,
                prio == ErrClass::PRIO_INFO ? ErrClass::NOERR : ErrClass::UNEXPECTEDSTATE,
                "RsMqttConnections", func, CharString(text.c_str()));
}

static int64_t nowMs()
{
  return std::chrono::duration_cast<std::chrono::milliseconds>(
           std::chrono::system_clock::now().time_since_epoch()).count();
}

RsMqttConnection::RsMqttConnection(const std::string &n)
  : name(n), settings(n, (int) Resources::getManNum()) {}

rsmqtt::Options RsMqttConnection::clientOptions()
{
  std::vector<std::string> warnings;
  rsmqtt::Options o = settings.clientOptions(RsMqttRedu::isHost2(), (const char *) Resources::getDataDir(), nowMs(),
                                             warnings);
  for (const auto &w : warnings)
    logError(ErrClass::PRIO_WARNING, "clientOptions", "[" + name + "] " + w);
  spbHost = settings.spbStateHost(RsMqttRedu::isHost2());
  return o;
}

bool RsMqttConnection::applyBrokerConfig(const RsMqttBrokerConfig &cfg, std::string &err)
{
  return settings.applyBrokerConfig(cfg, RsMqttRedu::isHost2(), RsMqttSecret::reveal, err);
}

//--------------------------------------------------------------------------------

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
      int q = 0, fl = 0;
      if ( !router.filter(filters[j], q, fl) ) continue;
      f.push_back(filters[j]);
      qos.push_back(q);
      flags.push_back(fl);
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
  subscribe(router.filters());
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
    // old object stays in brokerDps. A broker that left the list has none
    // (releaseBrokerDp) and gets a new one as well.
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

void RsMqttConnections::releaseBrokerDp(RsMqttConnection &conn)
{
  if ( !conn.brokerDp || conn.brokerDp->isDeleted() )
    return;
  for (auto it = brokerDps.begin(); it != brokerDps.end(); ++it)
  {
    if ( it->get() == conn.brokerDp )
    {
      conn.brokerDp = nullptr;
      brokerDps.erase(it);  // ~DrvIntDp: dpDisconnect per element
      return;
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

  std::string filter, err;
  switch ( conn->router.add(fullAddress, addr.dyn, input, conn->settings.subscribeQoS(), filter, err) )
  {
    case rsmqtt::wincc::Router::Added::State:
      conn->statePushNeeded = true;  // HWService writes the current state
      break;
    case rsmqtt::wincc::Router::Added::Subscribe:
      // (Re-)subscribe right away if connected; otherwise it happens on connect.
      conn->subscribe({filter});
      break;
    case rsmqtt::wincc::Router::Added::Error:
      logError(ErrClass::PRIO_SEVERE, "addAddress", "address '" + fullAddress + "': " + err);
      break;
    default:
      break;
  }
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

  std::string filter;
  if ( RsMqttConnection *conn = find(addr.connection) )
    if ( conn->router.remove(fullAddress, input, filter) )
      conn->unsubscribe(filter);
  return last;
}
