#include "RsMqttHWService.hxx"
#include "RsMqttBrokerDp.hxx"
#include "RsMqttConnections.hxx"
#include "RsMqttRedu.hxx"
#include "RsMqttResources.hxx"
#include "RsMqttService.hxx"
#include "RsMqttTime.hxx"
#include "RsMqttSparkplug.hxx"

#include <DrvManager.hxx>
#include <ErrHdl.hxx>
#include <HWObject.hxx>
#include <PVSSMacros.hxx>
#include <TimeVar.hxx>

#include <chrono>
#include <cstring>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

namespace
{
  using Clock = std::chrono::steady_clock;

  // Max. number of events handled per connection and workProc call, so a
  // flood of messages cannot block the driver's message handling.
  constexpr int MAX_EVENTS_PER_WORKPROC = 2000;

  // Config changes of a broker DP arrive element by element; wait this
  // long after the last one before re-creating the client.
  constexpr auto RECONFIGURE_DELAY = std::chrono::milliseconds(300);

  // State.MessagesIn/Out are written at most this often.
  constexpr auto COUNTER_INTERVAL = std::chrono::seconds(1);

  // Warn about addresses of unconfigured brokers after this time.
  constexpr auto PLACEHOLDER_WARN_DELAY = std::chrono::seconds(30);

  std::unique_ptr<RsMqttDriverIntDp> driverIntDp;  // lives until the driver exits

  void log(ErrClass::ErrPrio prio, const char *func, const std::string &text)
  {
    ErrHdl::error(prio, ErrClass::ERR_CONTROL,
                  prio == ErrClass::PRIO_INFO ? ErrClass::NOERR : ErrClass::UNEXPECTEDSTATE,
                  "RsMqttHWService", func, CharString(text.c_str()));
  }

  // ----- trace: the rsmqtt callback runs on library threads, so the lines
  // are buffered and written by workProc on the main thread.
  std::mutex                               traceMutex;
  std::deque<std::pair<int, std::string>>  traceLines;
  constexpr size_t                         MAX_TRACE_LINES = 10000;

  extern "C" void traceCallback(int level, const char *message, void *)
  {
    std::lock_guard<std::mutex> lock(traceMutex);
    if ( traceLines.size() < MAX_TRACE_LINES )
      traceLines.emplace_back(level, message ? message : "");
  }

  const char *opName(rsmqtt::Op op)
  {
    switch ( op )
    {
      case rsmqtt::Op::Connect:     return "connect";
      case rsmqtt::Op::Publish:     return "publish";
      case rsmqtt::Op::Subscribe:   return "subscribe";
      case rsmqtt::Op::Unsubscribe: return "unsubscribe";
      case rsmqtt::Op::Disconnect:  return "disconnect";
      default:                      return "operation";
    }
  }
}

//--------------------------------------------------------------------------------
// called after connect to data

PVSSboolean RsMqttHWService::initialize(int, char *[])
{
  if ( RsMqttResources::getTraceLevel() > 0 )
    rsmqtt_set_trace_callback(RsMqttResources::getTraceLevel(), traceCallback, nullptr);

  ErrHdl::error(ErrClass::PRIO_INFO, ErrClass::ERR_CONTROL, ErrClass::NOERR, "RsMqttHWService", "initialize",
                CharString(rsmqtt::version()));
  if ( RsMqttRedu::simulatedHost() )
    log(ErrClass::PRIO_WARNING, "initialize",
        "simulateReduHost = " + std::to_string(RsMqttRedu::simulatedHost()) + ": acting as host " +
        std::to_string(RsMqttRedu::simulatedHost()) + " of a redundant pair (for tests only), using " +
        (const char *) RsMqttResources::getDriverDp());

  // Broker datapoints: _RsMqttDriver_<num>.Brokers lists them.
  driverIntDp = std::make_unique<RsMqttDriverIntDp>(RsMqttResources::getDriverDp());

  // Brokers from the config file
  for (auto &c : RsMqttConnections::instance().all())
    if ( c->source == RsMqttConnection::Source::ConfigFile )
      createClient(*c);

  return PVSS_TRUE;
}

//--------------------------------------------------------------------------------
// called after connect to event

PVSSboolean RsMqttHWService::start()
{
  if ( RsMqttResources::getServiceApi() )
    RsMqttService::start();
  started = true;
  startTime = Clock::now();
  for (auto &c : RsMqttConnections::instance().all())
  {
    // start() after stop() (e.g. the driver was stopped and started again
    // without exiting): stop() closed all clients, so create them again.
    if ( c->stopped )
    {
      c->stopped = false;
      c->lastError.clear();
      if ( c->source == RsMqttConnection::Source::ConfigFile )
        createClient(*c);
      else if ( c->source == RsMqttConnection::Source::Datapoint )
      {
        c->dirty = true;
        c->dirtySince = startTime;
      }
    }
    pushState(*c);
    if ( c->source == RsMqttConnection::Source::ConfigFile )
      connect(*c);
  }
  return PVSS_TRUE;
}

//--------------------------------------------------------------------------------

void RsMqttHWService::stop()
{
  auto &conns = RsMqttConnections::instance().all();

  // Disconnect all at once, then wait: in-flight messages get up to 2 s.
  std::vector<std::pair<RsMqttConnection *, rsmqtt::Token>> pending;
  for (auto &c : conns)
  {
    if ( !c->client ) continue;
    sparkplugGoodbye(*c);
    try
    {
      pending.emplace_back(c.get(), c->client->disconnect(std::chrono::seconds(2)));
    }
    catch (const rsmqtt::Error &)
    {
      // not connected: nothing to do
    }
  }
  for (auto &p : pending)
    p.first->client->waitForCompletion(p.second, std::chrono::seconds(3));

  for (auto &c : conns)
  {
    c->client.reset();
    c->connected = false;
    c->connecting = false;
    c->stopped = true;
    if ( c->isActive() )
      c->lastError = "driver stopped";
    pushState(*c);
    sparkplugOffline(*c);
    publishBrokerState(*c, true);
  }
  // subscription changes of the last second: kept for the next start
  if ( RsMqttService *service = RsMqttService::get() )
    service->flushState();
  rsmqtt_set_trace_callback(0, nullptr, nullptr);
  drainTrace();
  started = false;
}

//--------------------------------------------------------------------------------

void RsMqttHWService::createClient(RsMqttConnection &conn)
{
  try
  {
    conn.client = std::make_unique<rsmqtt::Client>(conn.clientOptions());
    conn.lastError.clear();
    conn.configError = false;
    conn.everConnected = false;
    conn.messagesIn = conn.messagesOut = 0;
    DEBUG_DRV_USR1("rsmqtt [" << conn.name.c_str() << "] client " << conn.client->clientId().c_str() << " created");
  }
  catch (const rsmqtt::Error &e)
  {
    conn.client.reset();
    conn.lastError = e.what();
    conn.configError = true;
    log(ErrClass::PRIO_SEVERE, "createClient", "[" + conn.name + "] cannot create MQTT client: " + e.what());
  }
}

void RsMqttHWService::teardown(RsMqttConnection &conn)
{
  if ( conn.client )
  {
    sparkplugGoodbye(conn);
    try
    {
      rsmqtt::Token t = conn.client->disconnect(std::chrono::milliseconds(500));
      conn.client->waitForCompletion(t, std::chrono::seconds(1));
    }
    catch (const rsmqtt::Error &)
    {
    }
    conn.client.reset();
  }
  conn.connected = false;
  conn.connecting = false;
  conn.everConnected = false;
  conn.configError = false;
  conn.subscribeTokens.clear();
  conn.serverUri.clear();
  pushState(conn);
  sparkplugOffline(conn);
}

void RsMqttHWService::reconfigure(RsMqttConnection &conn)
{
  conn.dirty = false;
  conn.reconnectRequested = false;

  teardown(conn);
  conn.lastError.clear();

  if ( conn.source == RsMqttConnection::Source::Datapoint )
  {
    if ( !conn.isActive() )
    {
      log(ErrClass::PRIO_INFO, "reconfigure", "[" + conn.name + "] broker disabled");
      publishBrokerState(conn, true);
      // no longer listed: let go of its DP, which is usually deleted next
      if ( !conn.listed )
        RsMqttConnections::instance().releaseBrokerDp(conn);
      return;
    }
    std::string err;
    if ( !conn.applyBrokerConfig(conn.brokerDp->config(), err) )
    {
      conn.lastError = err;
      conn.configError = true;
      log(ErrClass::PRIO_SEVERE, "reconfigure", "[" + conn.name + "] invalid broker configuration: " + err);
      publishBrokerState(conn, true);
      return;
    }
    log(ErrClass::PRIO_INFO, "reconfigure", "[" + conn.name + "] broker configuration applied");
  }
  else if ( conn.source != RsMqttConnection::Source::ConfigFile )
    return;

  createClient(conn);
  connect(conn);
  publishBrokerState(conn, true);
}

//--------------------------------------------------------------------------------

void RsMqttHWService::connect(RsMqttConnection &conn)
{
  if ( !conn.client || conn.connected || conn.connecting )
    return;
  try
  {
    conn.client->connect();
    conn.connecting = true;
    DEBUG_DRV_USR1("rsmqtt [" << conn.name.c_str() << "] connecting");
  }
  catch (const rsmqtt::Error &e)
  {
    if ( e.code() == RSMQTT_CONNECT_IN_PROGRESS || e.code() == RSMQTT_ALREADY_CONNECTED )
      return;  // the library is (re)connecting by itself
    conn.lastError = e.what();
    conn.nextConnectAttempt = Clock::now() + conn.settings.reconnectInterval();
    log(ErrClass::PRIO_WARNING, "connect", "[" + conn.name + "] connect failed: " + e.what());
  }
}

//--------------------------------------------------------------------------------

void RsMqttHWService::workProc()
{
  drainTrace();
  RsMqttService *service = RsMqttService::get();
  if ( !started )
  {
    if ( service ) service->workProc();
    return;
  }

  // subscriptions restored after a restart: in the routers before messages come
  if ( service )
    service->attachRestored();

  auto now = Clock::now();
  for (auto &c : RsMqttConnections::instance().all())
  {
    if ( (c->dirty && now - c->dirtySince >= RECONFIGURE_DELAY) || c->reconnectRequested )
      reconfigure(*c);

    if ( c->statePushNeeded )
    {
      c->statePushNeeded = false;
      pushState(*c);
    }

    if ( c->client )
    {
      handleEvents(*c);

      // Retry connects that failed. Once a connection was established,
      // the library's automaticReconnect takes over (unless disabled).
      bool libraryReconnects = c->everConnected && c->settings.autoReconnect();
      if ( !c->connected && !c->connecting && !libraryReconnects &&
           now >= c->nextConnectAttempt && !c->client->isConnected() )
        connect(*c);
    }

    if ( c->source == RsMqttConnection::Source::Placeholder && !c->placeholderWarned &&
         now - startTime >= PLACEHOLDER_WARN_DELAY &&
         c->router.hasAddresses() )
    {
      c->placeholderWarned = true;
      log(ErrClass::PRIO_WARNING, "workProc", "addresses use broker '" + c->name +
          "', which is neither in the config file nor listed in " + (const char *) RsMqttResources::getDriverDp() + ".Brokers");
    }

    publishBrokerState(*c, false);
  }

  // after handleEvents: held Receive calls get this round's messages
  if ( service )
    service->workProc();
}

//--------------------------------------------------------------------------------

void RsMqttHWService::publishBrokerState(RsMqttConnection &conn, bool force)
{
  if ( !conn.brokerDp )
    return;
  // State/error changes go out at once; counters at most once per second.
  auto now = Clock::now();
  bool countersDue = force || now - conn.lastStatePublish >= COUNTER_INTERVAL;
  if ( countersDue )
    conn.lastStatePublish = now;

  std::string clientId = conn.client ? conn.client->clientId() : std::string();
  conn.brokerDp->publishState(conn.connState(), conn.lastError, conn.serverUri, clientId,
                              conn.messagesIn, conn.messagesOut, countersDue);
}

//--------------------------------------------------------------------------------

void RsMqttHWService::handleEvents(RsMqttConnection &conn)
{
  for (int i = 0; i < MAX_EVENTS_PER_WORKPROC; i++)
  {
    std::optional<rsmqtt::Event> ev;
    try
    {
      ev = conn.client->poll();
    }
    catch (const rsmqtt::Error &e)
    {
      log(ErrClass::PRIO_SEVERE, "workProc", "[" + conn.name + "] event poll failed: " + e.what());
      return;
    }
    if ( !ev ) return;

    switch ( ev->type() )
    {
      case rsmqtt::EventType::Message:
      {
        std::string topic(ev->topic());
        DEBUG_DRV_USR2("rsmqtt [" << conn.name.c_str() << "] message " << topic.c_str()
                       << " (" << (int) ev->payload().size() << " bytes)");
        conn.messagesIn++;
        deliverMessage(conn, topic, ev->payload(), ev->qos(), ev->retained());
        break;
      }

      case rsmqtt::EventType::Connected:
        conn.connected = true;
        conn.connecting = false;
        conn.everConnected = true;
        conn.lastError.clear();
        conn.serverUri = std::string(ev->serverUri());
        log(ErrClass::PRIO_INFO, "workProc", "[" + conn.name + "] connected to " + conn.serverUri +
            (ev->sessionPresent() ? " (session present)" : ""));
        pushState(conn);
        // Subscribing again also makes the broker resend retained values,
        // like a general query.
        conn.subscribeAll();
        sparkplugConnected(conn);
        break;

      case rsmqtt::EventType::ConnectionLost:
        conn.connected = false;
        conn.lastError = "connection lost";
        log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] connection lost");
        pushState(conn);
        sparkplugOffline(conn);
        break;

      case rsmqtt::EventType::Disconnected:
        conn.connected = false;
        conn.lastError = std::string(ev->text());
        log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] " + conn.lastError);
        pushState(conn);
        sparkplugOffline(conn);
        break;

      case rsmqtt::EventType::Complete:
      {
        if ( ev->op() == rsmqtt::Op::Connect )
        {
          conn.connecting = false;
          if ( !ev->ok() )
          {
            conn.lastError = std::string(ev->text());
            conn.nextConnectAttempt = Clock::now() + conn.settings.reconnectInterval();
            log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] connect failed: " + conn.lastError);
          }
          break;
        }

        if ( ev->op() == rsmqtt::Op::Subscribe )
        {
          auto it = conn.subscribeTokens.find(ev->token());
          if ( it != conn.subscribeTokens.end() )
          {
            std::vector<int> codes = ev->reasonCodes();
            for (size_t k = 0; k < it->second.size() && k < codes.size(); k++)
              if ( codes[k] >= 0x80 )
                log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] subscription '" + it->second[k] +
                    "' refused: " + rsmqtt::reasonCodeString(codes[k]));
            if ( !ev->ok() && codes.empty() )
              log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] subscribe failed: " + std::string(ev->text()));
            conn.subscribeTokens.erase(it);
          }
          break;
        }

        if ( !ev->ok() )
          log(ErrClass::PRIO_WARNING, "workProc", "[" + conn.name + "] " + opName(ev->op()) + " failed: " +
              std::string(ev->text()));
        break;
      }

      default:
        break;
    }
  }
}

//--------------------------------------------------------------------------------

void RsMqttHWService::deliver(const char *address, std::string_view payload, const TimeVar &time, bool periphTime,
                              int validity)
{
  HWObject obj;
  obj.setAddress(address);
  obj.setOrgTime(time);
  if ( periphTime )
    obj.setTimeOfPeriphFlag();  // otherwise WinCC OA stamps the value itself
  if ( validity == 2 )
    obj.setSbit(DRV_INVALID);
  else if ( validity == 1 )
    obj.clearSbit(DRV_INVALID);

  // HWObject owns and frees its buffer (delete[])
  PVSSchar *buf = new PVSSchar[payload.size() ? payload.size() : 1];
  if ( !payload.empty() )
    memcpy(buf, payload.data(), payload.size());
  obj.setData(buf);
  obj.setDlen((PVSSulong) payload.size());

  HWObject *addrObj = DrvManager::getHWMapperPtr()->findHWObject(&obj);
  if ( addrObj )
    DrvManager::getSelfPtr()->toDp(&obj, addrObj);
}


void RsMqttHWService::apply(RsMqttConnection &conn, const rsmqtt::wincc::Router::Outcome &out, std::string_view payload)
{
  for (const auto &w : out.warnings)
  {
    if ( w.first )
      log(ErrClass::PRIO_WARNING, "sparkplug", "[" + conn.name + "] " + w.second);
    else
      log(ErrClass::PRIO_WARNING, "deliver", w.second);
  }
  if ( conn.client )
  {
    for (const auto &s : out.subscribe)
    {
      try
      {
        conn.client->subscribe(std::string(s.first), s.second);
      }
      catch (const rsmqtt::Error &e)
      {
        log(ErrClass::PRIO_WARNING, "sparkplug", "[" + conn.name + "] subscribe '" + s.first + "': " + e.what());
      }
    }
    for (const auto &p : out.publish)
    {
      try
      {
        conn.client->publish(p.topic, p.payload, p.qos, p.retain);
      }
      catch (const rsmqtt::Error &e)
      {
        log(ErrClass::PRIO_WARNING, "sparkplug", "[" + conn.name + "] " + p.topic + ": " + e.what());
      }
    }
  }
  for (const auto &d : out.deliveries)
    deliver(d.address, d.hasValue ? d.value : payload, d.hasTime ? rsmqttTimeOfMs(d.timeMs) : TimeVar(), d.periphTime,
            d.validity);
}

void RsMqttHWService::deliverMessage(RsMqttConnection &conn, const std::string &topic, std::string_view payload,
                                     int qos, bool retained)
{
  const auto &out = conn.router.message(topic, payload, rsmqttNowMs(), conn.spbStateHost(), conn.settings.stateTs(),
                                        retained);
  apply(conn, out, payload);
  // the MSA service subscriptions the router found for it
  if ( out.serviceCount )
    if ( RsMqttService *service = RsMqttService::get() )
      service->onMessage(out, topic, payload, qos, retained);
}

//--------------------------------------------------------------------------------
// Sparkplug B host application

void RsMqttHWService::sparkplugConnected(RsMqttConnection &conn)
{
  // the host application is online: subscriptions first, then STATE (Sparkplug 3.0);
  // edge nodes and devices count as offline until their birth
  if ( conn.client )
    apply(conn, conn.router.connected(conn.spbStateHost(), conn.settings.stateTs()));
}

void RsMqttHWService::sparkplugGoodbye(RsMqttConnection &conn)
{
  // a clean disconnect sends no will: the host application says offline itself
  if ( conn.spbStateHost().empty() || !conn.client || !conn.connected )
    return;
  try
  {
    conn.client->publish(RsMqttSparkplug::stateTopic(conn.spbStateHost()),
                         RsMqttSparkplug::statePayload(false, conn.settings.stateTs()), 1, true);
  }
  catch (const rsmqtt::Error &)
  {
  }
}

void RsMqttHWService::sparkplugOffline(RsMqttConnection &conn)
{
  apply(conn, conn.router.connectionLost());
}

void RsMqttHWService::pushState(RsMqttConnection &conn)
{
  apply(conn, conn.router.state(conn.connected));
}

//--------------------------------------------------------------------------------

void RsMqttHWService::drainTrace()
{
  std::deque<std::pair<int, std::string>> lines;
  {
    std::lock_guard<std::mutex> lock(traceMutex);
    lines.swap(traceLines);
  }
  for (const auto &l : lines)
  {
    ErrClass::ErrPrio prio = l.first <= 1 ? ErrClass::PRIO_SEVERE
                           : l.first == 2 ? ErrClass::PRIO_WARNING
                                          : ErrClass::PRIO_INFO;
    ErrHdl::error(prio, ErrClass::ERR_CONTROL, ErrClass::NOERR, "rsmqtt", "trace", CharString(l.second.c_str()));
  }
}

//--------------------------------------------------------------------------------
// we get data from WinCC OA and send it to the broker

PVSSboolean RsMqttHWService::writeData(HWObject *objPtr)
{
  const std::string name((const char *) objPtr->getAddress());
  RsMqttConnections &conns = RsMqttConnections::instance();

  const RsMqttAddress *addr = conns.parsed(name);
  RsMqttConnection *conn = addr ? conns.find(addr->connection) : nullptr;
  if ( !addr || !conn || !conn->client )
  {
    log(ErrClass::PRIO_WARNING, "writeData", "no active MQTT connection for address '" + name + "'");
    return PVSS_FALSE;
  }

  // RsMqttTrans::toPeriph: 4 byte little endian length + payload
  const PVSSchar *data = objPtr->getData();
  PVSSulong dlen = objPtr->getDlen();
  if ( !data || dlen < 4 )
  {
    log(ErrClass::PRIO_WARNING, "writeData", "no data for address '" + name + "'");
    return PVSS_FALSE;
  }
  uint32_t n = (uint32_t) data[0] | ((uint32_t) data[1] << 8) | ((uint32_t) data[2] << 16) | ((uint32_t) data[3] << 24);
  if ( n > dlen - 4 )
  {
    log(ErrClass::PRIO_WARNING, "writeData", "corrupt data for address '" + name + "'");
    return PVSS_FALSE;
  }
  std::string_view value(reinterpret_cast<const char *>(data + 4), n);

  // topic, payload (json= / ts=, Sparkplug NCMD / DCMD), QoS and retain:
  // crates/rsmqtt-wincc/src/router.rs (Router::write)
  const TimeVar &tm = objPtr->getOrgTime();
  int64_t timeMs = (int64_t) tm.getSeconds() * 1000 + tm.getMilli();
  rsmqtt::wincc::Router::Write w;
  std::string err;
  if ( !conn->router.write(name, value, (int) objPtr->getType(), timeMs, rsmqttNowMs(), conn->settings.publishQoS(),
                           conn->settings.publishRetain(), w, err) )
  {
    log(ErrClass::PRIO_WARNING, "writeData", err);
    return PVSS_FALSE;
  }

  if ( !addr->sparkplug && Resources::isDbgFlag(Resources::DBG_DRV_USR1) )
    objPtr->debugPrint();

  if ( !conn->publish(w.topic, w.payload, w.qos, w.retain, err) )
  {
    log(ErrClass::PRIO_WARNING, "writeData", "[" + conn->name + "] " +
        (addr->sparkplug ? w.topic + ": " : "publish to '" + w.topic + "' failed: ") + err);
    return PVSS_FALSE;
  }
  return PVSS_TRUE;
}
