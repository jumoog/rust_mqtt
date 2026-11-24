#include "RsMqttService.hxx"
#include "RsMqttAddress.hxx"
#include "RsMqttConnections.hxx"
#include "RsMqttRedu.hxx"
#include "RsMqttResources.hxx"
#include "RsMqttTime.hxx"

#include <BitVar.hxx>
#include <BlobVar.hxx>
#include <DrvManager.hxx>
#include <DrvResources.hxx>
#include <DynVar.hxx>
#include <ErrHdl.hxx>
#include <FloatVar.hxx>
#include <IntegerVar.hxx>
#include <MappingVar.hxx>
#include <PVSSMacros.hxx>
#include <Resources.hxx>
#include <TextVar.hxx>

#include <algorithm>
#include <cstdlib>
#include <filesystem>
#include <fstream>

using namespace std::placeholders;

namespace
{
  // A subscription without a Receive or Renew for this long is dropped.
  constexpr auto LEASE = std::chrono::seconds(120);
  // Messages and payload bytes kept per subscription between two Receive
  // calls (beyond either, the oldest are dropped).
  constexpr size_t MAX_QUEUE = 10000;
  constexpr size_t MAX_QUEUE_BYTES = 64u * 1024u * 1024u;
  // Receive: default and maximum of `wait` (s) and `max`.
  constexpr double   DEFAULT_WAIT = 10.0, MAX_WAIT = 300.0;
  constexpr unsigned DEFAULT_MAX = 100, MAX_MAX = 10000;
  // A held Receive is answered this long before the call's deadline.
  constexpr long long DEADLINE_MARGIN_MS = 1000;
  // Deadlines, cancellations and leases are checked this often.
  constexpr auto SCAN_INTERVAL = std::chrono::milliseconds(100);
  // The state file is written at most this often (and again after a failure).
  constexpr auto SAVE_INTERVAL = std::chrono::seconds(1);

  std::unique_ptr<vrpc::ServiceContainer> container;

  void log(ErrClass::ErrPrio prio, const char *func, const std::string &text)
  {
    ErrHdl::error(prio, ErrClass::ERR_CONTROL,
                  prio == ErrClass::PRIO_INFO ? ErrClass::NOERR : ErrClass::UNEXPECTEDSTATE,
                  "RsMqttService", func, CharString(text.c_str()));
  }

  // ----- request fields

  const Variable *field(const Variable *req, const char *key)
  {
    if ( !req || req->isA() != MAPPING_VAR )
      return nullptr;
    return static_cast<const MappingVar *>(req)->getAt(TextVar(key));
  }

  bool getString(const Variable *req, const char *key, std::string &out)
  {
    const Variable *v = field(req, key);
    if ( !v ) return false;
    TextVar t;
    t = *v;
    out = t.getValue() ? t.getValue() : "";
    return true;
  }

  bool getInt(const Variable *req, const char *key, int &out)
  {
    const Variable *v = field(req, key);
    if ( !v ) return false;
    IntegerVar i;
    i = *v;
    out = i.getValue();
    return true;
  }

  bool getDouble(const Variable *req, const char *key, double &out)
  {
    const Variable *v = field(req, key);
    if ( !v ) return false;
    FloatVar f;
    f = *v;
    out = f.getValue();
    return true;
  }

  bool getBool(const Variable *req, const char *key, bool def)
  {
    const Variable *v = field(req, key);
    if ( !v ) return def;
    BitVar b;
    b = *v;
    return b.getValue() != 0;
  }

  vrpc::Status status(vrpc::StatusCode code, const std::string &text)
  {
    return vrpc::Status(code, CharString(text.c_str()));
  }

  vrpc::Status invalid(const std::string &text) { return status(vrpc::StatusCode::InvalidArgument, text); }
  vrpc::Status notFound(const std::string &text) { return status(vrpc::StatusCode::NotFound, text); }

  /// "qos" of a request (default `def`); false: not 0..2.
  bool getQoS(const Variable *req, int def, int &qos)
  {
    qos = def;
    getInt(req, "qos", qos);
    return qos >= 0 && qos <= 2;
  }

  /// Adds key -> value to a mapping, which takes the value.
  void put(MappingVar &m, const char *key, Variable *value)
  {
    m.setAt(new TextVar(key), value);
  }

  /// A response {key: value}.
  vrpc::Status reply(Variable *&resp, const char *key, Variable *value)
  {
    MappingVar *m = new MappingVar;
    put(*m, key, value);
    resp = m;
    return vrpc::Status::OK;
  }

  /// A configured broker connection (not a placeholder of addresses).
  RsMqttConnection *findBroker(const std::string &broker)
  {
    RsMqttConnection *conn = RsMqttConnections::instance().find(broker);
    if ( !conn || conn->source == RsMqttConnection::Source::Placeholder )
      return nullptr;
    return conn;
  }
}

std::shared_ptr<RsMqttService> RsMqttService::self;

//--------------------------------------------------------------------------------

RsMqttService::RsMqttService()
  : vrpc::ServiceBase<RsMqttService>("RsMqtt")
{
  char buf[32];
  snprintf(buf, sizeof(buf), "%llx-", (unsigned long long) rsmqttNowMs());
  idPrefix = buf;
  startedAt = Clock::now();

  std::string file = "service_" + std::to_string((int) Resources::getManNum()) + (RsMqttRedu::isHost2() ? "_2" : "") +
                     ".state";
  statePath = (std::filesystem::path((const char *) Resources::getDataDir()) / "rsmqtt" / file).string();
  loadState();

  registerFunction("Subscribe",   msa::VrpcUnaryMethodSyncDelegate(std::bind(&RsMqttService::subscribe, this, _1, _2, _3)));
  registerFunction("Unsubscribe", msa::VrpcUnaryMethodSyncDelegate(std::bind(&RsMqttService::unsubscribe, this, _1, _2, _3)));
  registerFunction("Renew",       msa::VrpcUnaryMethodSyncDelegate(std::bind(&RsMqttService::renew, this, _1, _2, _3)));
  registerFunction("Publish",     msa::VrpcUnaryMethodSyncDelegate(std::bind(&RsMqttService::publish, this, _1, _2, _3)));
  registerFunction("Receive",     msa::VrpcUnaryMethodAsyncDelegate(std::bind(&RsMqttService::receive, this, _1, _2, _3)));
}

void RsMqttService::start()
{
  if ( self )
    return;
  self = std::make_shared<RsMqttService>();

  vrpc::ServiceOptions options;
  options.withManagerSpecificService();  // this driver: DRIVER_MAN, -num

  container = std::make_unique<vrpc::ServiceContainer>();
  container->registerService(self, options);
  container->startAllServices([](bool ok)
  {
    if ( ok )
      log(ErrClass::PRIO_INFO, "start", "service RsMqtt started (vRPC, driver " +
          std::to_string((int) Resources::getManNum()) + ")");
    else
      log(ErrClass::PRIO_SEVERE, "start", "service RsMqtt could not be started");
  });
  self->attachRestored();  // brokers of the config file are there already
}

//--------------------------------------------------------------------------------
// state file

void RsMqttService::loadState()
{
  namespace svc = rsmqtt::wincc::service;
  std::ifstream in(std::filesystem::path(statePath), std::ios::binary);
  if ( !in )
    return;
  std::string line;
  bool header = false;
  size_t restored = 0, skipped = 0;
  while ( std::getline(in, line) )
  {
    svc::Record r;
    svc::Line kind = svc::parse(line, r);
    if ( kind == svc::Line::Header )
      header = true;
    else if ( kind == svc::Line::Record && header && !byName.count(r.id) )
    {
      uint64_t id = ++nextId;
      Subscription &sub = subs[id];
      sub.name = r.id;
      sub.broker = r.broker;
      sub.filter = r.filter;
      sub.qos = r.qos;
      sub.binary = r.binary;
      sub.restored = true;
      sub.lastSeen = Clock::now();
      byName[r.id] = id;
      unattached++;
      restored++;
    }
    else if ( !line.empty() && line != "\r" )
      skipped++;
  }
  if ( restored || skipped )
    log(skipped ? ErrClass::PRIO_WARNING : ErrClass::PRIO_INFO, "loadState",
        std::to_string(restored) + " subscription(s) restored from " + statePath +
        (skipped ? ", " + std::to_string(skipped) + " line(s) skipped" : ""));
}

bool RsMqttService::saveState()
{
  namespace fs = std::filesystem;
  namespace svc = rsmqtt::wincc::service;
  std::string text = svc::header() + "\n";
  for (const auto &[id, sub] : subs)
    text += svc::record({sub.name, sub.qos, sub.binary, sub.broker, sub.filter}) + "\n";

  // a new file, then rename: a crash never leaves half a file
  fs::path path(statePath), tmp(statePath + ".tmp");
  std::error_code ec;
  fs::create_directories(path.parent_path(), ec);
  if ( !ec )
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    out << text;
    out.close();
    if ( !out )
      ec = std::make_error_code(std::errc::io_error);
  }
  if ( !ec )
    fs::rename(tmp, path, ec);
  if ( ec && !stateWarned )
    log(ErrClass::PRIO_WARNING, "saveState", "cannot write " + statePath + " (" + ec.message() +
        "): subscriptions are not resumed after a restart of the driver");
  else if ( !ec && stateWarned )
    log(ErrClass::PRIO_INFO, "saveState", statePath + " written again");
  stateWarned = (bool) ec;
  return !ec;
}

void RsMqttService::flushState()
{
  if ( stateDirty )
    stateDirty = !saveState();
}

void RsMqttService::attachRestored()
{
  if ( !unattached )
    return;
  auto now = Clock::now();
  for (auto it = subs.begin(); it != subs.end();)
  {
    Subscription &sub = it->second;
    if ( sub.conn )
    {
      ++it;
      continue;
    }
    RsMqttConnection *conn = findBroker(sub.broker);
    std::string filter, err;
    if ( conn && conn->router.addService(it->first, sub.filter, sub.qos, filter, err) )
    {
      sub.conn = conn;
      unattached--;
      conn->subscribe({filter});  // or on connect
      DEBUG_DRV_USR1("rsmqtt service: subscription " << sub.name.c_str() << " " << sub.broker.c_str() << ":"
                     << sub.filter.c_str() << " restored");
    }
    else if ( conn || now - startedAt > LEASE )
    {
      log(ErrClass::PRIO_WARNING, "attachRestored", "restored subscription " + sub.name + " (" + sub.broker + ":" +
          sub.filter + ") dropped: " + (conn ? err : std::string("the broker is not configured")));
      release(it->first, sub);
      byName.erase(sub.name);
      it = subs.erase(it);
      stateDirty = true;
      continue;
    }
    ++it;
  }
}

RsMqttService::Subscription *RsMqttService::find(const Variable *req, uint64_t *idOut)
{
  std::string name;
  if ( !getString(req, "id", name) )
    return nullptr;
  auto n = byName.find(name);
  if ( n == byName.end() )
    return nullptr;
  auto it = subs.find(n->second);
  if ( it == subs.end() )
    return nullptr;
  if ( idOut ) *idOut = n->second;
  return &it->second;
}

void RsMqttService::erase(uint64_t id)
{
  auto it = subs.find(id);
  if ( it == subs.end() )
    return;
  byName.erase(it->second.name);
  subs.erase(it);
}

//--------------------------------------------------------------------------------
// Subscribe {broker, topic, qos?, binary?} -> {id}

vrpc::Status RsMqttService::subscribe(std::shared_ptr<vrpc::ServerContext>, const Variable *req, Variable *&resp)
{
  std::string broker, topic;
  if ( !getString(req, "broker", broker) || !getString(req, "topic", topic) )
    return invalid("Subscribe needs a mapping with \"broker\" and \"topic\"");
  RsMqttConnection *conn = findBroker(broker);
  if ( !conn )
    return notFound("unknown broker '" + broker + "'");
  int qos;
  if ( !getQoS(req, conn->settings.subscribeQoS(), qos) )
    return invalid("qos must be 0, 1 or 2");

  // a restored id of an earlier run could have the same name (clock set back)
  uint64_t id;
  do
    id = ++nextId;
  while ( byName.count(idPrefix + std::to_string(id)) );
  std::string filter, err;
  if ( !conn->router.addService(id, topic, qos, filter, err) )
    return invalid("'" + topic + "': " + err);
  conn->subscribe({filter});  // also for a filter in use: the broker sends the retained messages

  Subscription &sub = subs[id];
  sub.conn = conn;
  sub.name = idPrefix + std::to_string(id);
  sub.broker = broker;
  sub.filter = topic;
  sub.qos = qos;
  sub.binary = getBool(req, "binary", false);
  sub.lastSeen = Clock::now();
  byName[sub.name] = id;
  stateDirty = true;
  DEBUG_DRV_USR1("rsmqtt service: subscription " << sub.name.c_str() << " " << broker.c_str() << ":" << topic.c_str());
  return reply(resp, "id", new TextVar(sub.name.c_str()));
}

//--------------------------------------------------------------------------------
// Unsubscribe {id} -> {ok}

vrpc::Status RsMqttService::unsubscribe(std::shared_ptr<vrpc::ServerContext>, const Variable *req, Variable *&resp)
{
  uint64_t id = 0;
  Subscription *sub = find(req, &id);
  if ( sub )
  {
    release(id, *sub);
    erase(id);
    stateDirty = true;
  }
  return reply(resp, "ok", new BitVar(sub != nullptr));
}

//--------------------------------------------------------------------------------
// Renew {id} -> {ok}: keeps a subscription while the client is busy with callbacks

vrpc::Status RsMqttService::renew(std::shared_ptr<vrpc::ServerContext>, const Variable *req, Variable *&resp)
{
  Subscription *sub = find(req);
  if ( !sub )
    return notFound("unknown subscription");
  sub->lastSeen = Clock::now();
  sub->restored = false;
  return reply(resp, "ok", new BitVar(true));
}

//--------------------------------------------------------------------------------
// Publish {broker, topic, payload (string or blob), qos?, retain?} -> {ok}

vrpc::Status RsMqttService::publish(std::shared_ptr<vrpc::ServerContext>, const Variable *req, Variable *&resp)
{
  std::string broker, topic, payload;
  if ( !getString(req, "broker", broker) || !getString(req, "topic", topic) || topic.empty() )
    return invalid("Publish needs a mapping with \"broker\", \"topic\" and \"payload\"");
  if ( RsMqttAddress::isWildcardFilter(topic) )
    return invalid("cannot publish to the filter '" + topic + "'");
  // like the driver's outputs: the passive redundancy host does not write
  if ( DrvResources::getDisableCommands() && !DrvResources::getPassiveDriverWrites() )
    return status(vrpc::StatusCode::FailedPrecondition, "this driver is the passive redundancy host");
  const Variable *p = field(req, "payload");
  if ( p && p->isA() == BLOB_VAR )
  {
    const Blob &b = static_cast<const BlobVar *>(p)->getValue();
    payload.assign(reinterpret_cast<const char *>(b.getData()), b.getLen());
  }
  else if ( p )
    getString(req, "payload", payload);
  if ( payload.size() > RsMqttResources::getMaxPayload() )
    return invalid("payload of " + std::to_string(payload.size()) + " bytes exceeds maxPayload " +
                   std::to_string(RsMqttResources::getMaxPayload()));

  RsMqttConnection *conn = findBroker(broker);
  if ( !conn )
    return notFound("unknown broker '" + broker + "'");
  int qos;
  if ( !getQoS(req, conn->settings.publishQoS(), qos) )
    return invalid("qos must be 0, 1 or 2");
  std::string err;
  if ( !conn->publish(topic, payload, qos, getBool(req, "retain", conn->settings.publishRetain()), err) )
    return status(vrpc::StatusCode::Unavailable, "[" + broker + "] " + topic + ": " + err);
  return reply(resp, "ok", new BitVar(true));
}

//--------------------------------------------------------------------------------
// Receive {id, wait?, max?} -> {messages: dyn_mapping, dropped}

void RsMqttService::receive(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req,
                            std::shared_ptr<vrpc::FunctionResponseSender> sender)
{
  // canceled before it got here (e.g. its deadline passed on the way): no need to hold it
  if ( ctx->isCanceled() )
  {
    sender->sendErrorResponse(vrpc::StatusCode::Canceled, "Receive was canceled");
    return;
  }
  Subscription *sub = find(req);
  if ( !sub )
  {
    sender->sendErrorResponse(notFound("unknown subscription"));
    return;
  }
  sub->restored = false;

  double wait = DEFAULT_WAIT;
  getDouble(req, "wait", wait);
  wait = (std::min)((std::max)(wait, 0.0), MAX_WAIT);
  int max = (int) DEFAULT_MAX;
  getInt(req, "max", max);
  unsigned umax = (unsigned) (std::min)((std::max)(max, 1), (int) MAX_MAX);

  // only one Receive per subscription: an older one gets what is there
  if ( sub->held )
    finishHeld(*sub);

  // answer before the client gives up
  long long waitMs = (long long) (wait * 1000);
  const PVSSTime &deadline = ctx->getDeadline();
  if ( deadline.getSeconds() > 0 && deadline < PVSSTime::MAX_TIME )
  {
    PVSSTime now = PVSSTime::getSystemTime();
    long long left = ((long long) deadline.getSeconds() - (long long) now.getSeconds()) * 1000 +
                     (deadline.getMilli() - now.getMilli()) - DEADLINE_MARGIN_MS;
    waitMs = (std::max)(0LL, (std::min)(waitMs, left));
  }

  if ( !sub->queue.empty() || sub->dropped || waitMs == 0 )
    answer(*sub, *sender, umax);
  else
    sub->held = Held{std::move(ctx), std::move(sender), Clock::now() + std::chrono::milliseconds(waitMs), umax};
}

void RsMqttService::answer(Subscription &sub, vrpc::FunctionResponseSender &sender, unsigned max)
{
  // the mappings take the Variables: nothing is copied twice
  DynVar *messages = new DynVar(MAPPING_VAR);
  for (unsigned n = 0; n < max && !sub.queue.empty(); n++)
  {
    const Message &msg = *sub.queue.front();
    MappingVar *m = new MappingVar;
    put(*m, "topic", new TextVar(msg.topic.c_str()));
    // a string, unless blobs were asked for or the payload cannot be a CTRL string
    bool blob = sub.binary || !msg.text;
    if ( blob )
      put(*m, "payload", new BlobVar((PVSSuchar *) msg.payload.data(), (PVSSulong) msg.payload.size(), true));
    else
      put(*m, "payload", new TextVar(msg.payload.c_str()));
    put(*m, "binary", new BitVar(blob));
    put(*m, "qos", new IntegerVar(msg.qos));
    put(*m, "retained", new BitVar(msg.retained));
    put(*m, "time", new TimeVar(rsmqttTimeOfMs(msg.timeMs)));
    messages->append(m);
    sub.queueBytes -= msg.payload.size();
    sub.queue.pop_front();
  }
  MappingVar resp;
  put(resp, "messages", messages);
  put(resp, "dropped", new IntegerVar((int) sub.dropped));
  sub.dropped = 0;
  sub.lastSeen = Clock::now();
  sender.sendSuccessResponse(&resp);
}

void RsMqttService::finishHeld(Subscription &sub)
{
  Held h = std::move(*sub.held);
  sub.held.reset();
  answer(sub, *h.sender, h.max);
}

void RsMqttService::release(uint64_t id, Subscription &sub)
{
  if ( sub.held )
  {
    Held h = std::move(*sub.held);
    sub.held.reset();
    answer(sub, *h.sender, 0);  // the client's Receive returns, empty
  }
  if ( !sub.conn )  // restored, its broker is not there (yet)
  {
    unattached--;
    return;
  }
  std::string filter;
  if ( sub.conn->router.removeService(id, filter) )
    sub.conn->unsubscribe(filter);
}

//--------------------------------------------------------------------------------

void RsMqttService::onMessage(const rsmqtt::wincc::Router::Outcome &out, const std::string &topic,
                              std::string_view payload, int qos, bool retained)
{
  MessagePtr msg;  // made for the first subscription, shared by the others
  for (size_t i = 0; i < out.serviceCount; i++)
  {
    auto it = subs.find(out.service[i]);
    if ( it == subs.end() )
      continue;
    Subscription &sub = it->second;
    if ( !msg )
      msg = std::make_shared<const Message>(Message{topic, std::string(payload), qos, retained, out.serviceText, rsmqttNowMs()});
    while ( !sub.queue.empty() &&
            (sub.queue.size() >= MAX_QUEUE || sub.queueBytes + payload.size() > MAX_QUEUE_BYTES) )
    {
      sub.queueBytes -= sub.queue.front()->payload.size();
      sub.queue.pop_front();
      sub.dropped++;
    }
    sub.queue.push_back(msg);
    sub.queueBytes += payload.size();
    if ( sub.held )
      ready.push_back(it->first);
  }
}

void RsMqttService::workProc()
{
  // held Receives that got messages: answer them now
  for (uint64_t id : ready)
  {
    auto it = subs.find(id);
    if ( it != subs.end() && it->second.held )
      finishHeld(it->second);
  }
  ready.clear();

  auto now = Clock::now();
  // the state file: changes are collected, a failed write is tried again
  if ( stateDirty && now >= nextSave )
  {
    stateDirty = !saveState();
    nextSave = now + SAVE_INTERVAL;
  }
  if ( subs.empty() || now < nextScan )
    return;
  nextScan = now + SCAN_INTERVAL;

  attachRestored();
  for (auto it = subs.begin(); it != subs.end();)
  {
    Subscription &sub = it->second;
    if ( sub.held )
    {
      if ( sub.held->ctx && sub.held->ctx->isCanceled() )
      {
        sub.held->sender->sendErrorResponse(vrpc::StatusCode::Canceled, "Receive was canceled");
        sub.held.reset();
        sub.lastSeen = now;
      }
      else if ( !sub.queue.empty() || sub.dropped || now >= sub.held->until )
        finishHeld(sub);
    }
    else if ( now - sub.lastSeen > LEASE )
    {
      // a restored one no client took up (its manager ended meanwhile) ends normally
      size_t lost = sub.queue.size() + sub.dropped;
      log(lost && !sub.restored ? ErrClass::PRIO_WARNING : ErrClass::PRIO_INFO, "workProc",
          (sub.restored ? "restored subscription " : "subscription ") + sub.name + " (" + sub.broker + ":" +
          sub.filter + ") dropped: " + (sub.restored ? "not resumed" : "no Receive or Renew") + " for " +
          std::to_string(LEASE.count()) + " s" +
          (lost ? ", " + std::to_string(lost) + " message(s) " + (sub.restored ? "discarded" : "lost") : ""));
      release(it->first, sub);
      byName.erase(sub.name);
      it = subs.erase(it);
      stateDirty = true;
      continue;
    }
    ++it;
  }
}
