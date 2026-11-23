#include "RsMqttBrokerDp.hxx"
#include "RsMqttRedu.hxx"
#include "RsMqttConnections.hxx"

#include <BitVar.hxx>
#include <DynVar.hxx>
#include <ErrClass.hxx>
#include <ErrHdl.hxx>
#include <IntegerVar.hxx>
#include <Manager.hxx>
#include <PVSSMacros.hxx>
#include <TextVar.hxx>
#include <UIntegerVar.hxx>

#include <chrono>

namespace
{
  std::string toString(const Variable *v)
  {
    if ( !v ) return std::string();
    if ( v->isA() == TEXT_VAR )
    {
      const CharString &s = static_cast<const TextVar *>(v)->getString();
      return std::string((const char *) s, s.len());
    }
    CharString s = v->formatValue();
    return std::string((const char *) s, s.len());
  }

  int toInt(const Variable *v)
  {
    if ( !v ) return 0;
    Variable *out = nullptr;
    int result = 0;
    if ( v->convert(INTEGER_VAR, out) == Variable::OK && out )
      result = (int) static_cast<IntegerVar *>(out)->getValue();
    delete out;
    return result;
  }

  bool toBool(const Variable *v)
  {
    if ( !v ) return false;
    return v->isTrue() != 0;
  }

  std::vector<std::string> toStrings(const Variable *v)
  {
    std::vector<std::string> out;
    if ( !v ) return out;
    if ( !v->isDynVar() )
    {
      std::string s = toString(v);
      if ( !s.empty() ) out.push_back(s);
      return out;
    }
    const DynVar *dyn = static_cast<const DynVar *>(v);
    for (unsigned i = 0; i < dyn->size(); i++)
    {
      std::string s = toString(dyn->getAt(i));
      if ( !s.empty() ) out.push_back(s);
    }
    return out;
  }

  void markDirty(const std::string &broker)
  {
    if ( RsMqttConnection *c = RsMqttConnections::instance().find(broker) )
    {
      c->dirty = true;
      c->dirtySince = std::chrono::steady_clock::now();
    }
  }
}

//================================================================================
// _RsMqttDriver_<num>

RsMqttDriverIntDp::RsMqttDriverIntDp(const CharString &name)
  : DrvIntDp(maxDP, maxConnect), dpName(name)
{
  names[inBrokers]  = dpName + ".Brokers:_original.._value";
  names[outVersion] = dpName + ".Version:_original.._value";
}

const CharString &RsMqttDriverIntDp::getDpName4Query(int index)
{
  static CharString none;
  return (index >= 0 && index < maxDP) ? names[index] : none;
}

void RsMqttDriverIntDp::answer4DpId(int index, Variable *varPtr)
{
  if ( index == inBrokers )
    handleBrokers(varPtr);
  delete varPtr;
}

void RsMqttDriverIntDp::hotLink2Internal(int index, Variable *varPtr)
{
  if ( index == inBrokers )
    handleBrokers(varPtr);
  delete varPtr;
}

void RsMqttDriverIntDp::handleBrokers(const Variable *varPtr)
{
  std::vector<std::string> brokers = toStrings(varPtr);
  DEBUG_DRV_USR1("rsmqtt: " << (const char *) dpName << ".Brokers has " << (int) brokers.size() << " entries");
  RsMqttConnections::instance().setBrokerList(brokers);
}

void RsMqttDriverIntDp::dpReady()
{
  if ( getError() )
  {
    ErrHdl::error(ErrClass::PRIO_INFO, ErrClass::ERR_PARAM, ErrClass::NOERR, "RsMqttDriverIntDp", "dpReady",
                  CharString("datapoint ") + dpName + " (type _RsMqttDriver) not usable; only brokers from the config file are used" +
                  (RsMqttRedu::isHost2() ? " (redundant host 2: create the _2 datapoints with rsmqttUpgradeToRedundant())" : ""));
    return;
  }
  if ( isIdValid(outVersion) )
    Manager::dpSet(getId(outVersion), TextVar(rsmqtt::version()));
}

//================================================================================
// _RsMqtt_<broker>

CharString RsMqttBrokerIntDp::dpNameOf(const std::string &broker)
{
  return CharString(RsMqttRedu::forHost("_RsMqtt_" + broker, RsMqttRedu::isHost2()).c_str());
}

RsMqttBrokerIntDp::RsMqttBrokerIntDp(const std::string &b)
  : DrvIntDp(maxDP, maxConnect), broker(b)
{
  static const char *const elements[maxDP] = {
    "Config.Active", "Config.ServerURI", "Config.ServerURIs", "Config.ClientId", "Config.MQTTVersion",
    "Config.KeepAlive", "Config.CleanSession", "Config.Username", "Config.Password",
    "Config.TlsTrustStore", "Config.TlsKeyStore", "Config.TlsPrivateKey", "Config.TlsVerify",
    "Config.WillTopic", "Config.WillPayload", "Config.WillQoS", "Config.WillRetain",
    "Config.SubscribeQoS", "Config.PublishQoS", "Config.PublishRetain", "Config.Options",
    "Command.Reconnect",
    "State.ConnState", "State.LastError", "State.ServerURI", "State.ClientId",
    "State.MessagesIn", "State.MessagesOut",
  };
  CharString dp = dpNameOf(broker);
  for (int i = 0; i < maxDP; i++)
    names[i] = dp + "." + elements[i] + ":_original.._value";
}

const CharString &RsMqttBrokerIntDp::getDpName4Query(int index)
{
  static CharString none;
  return (index >= 0 && index < maxDP) ? names[index] : none;
}

void RsMqttBrokerIntDp::setValue(int index, const Variable *v)
{
  switch ( index )
  {
    case inActive:        cfg.active = toBool(v); break;
    case inServerURI:     cfg.serverURI = toString(v); break;
    case inServerURIs:    cfg.serverURIs = toStrings(v); break;
    case inClientId:      cfg.clientId = toString(v); break;
    case inMQTTVersion:   cfg.mqttVersion = toInt(v); break;
    case inKeepAlive:     cfg.keepAlive = toInt(v); break;
    case inCleanSession:  cfg.cleanSession = toBool(v); break;
    case inUsername:      cfg.username = toString(v); break;
    case inPassword:      cfg.password = toString(v); break;
    case inTlsTrustStore: cfg.tlsTrustStore = toString(v); break;
    case inTlsKeyStore:   cfg.tlsKeyStore = toString(v); break;
    case inTlsPrivateKey: cfg.tlsPrivateKey = toString(v); break;
    case inTlsVerify:     cfg.tlsVerify = toBool(v); break;
    case inWillTopic:     cfg.willTopic = toString(v); break;
    case inWillPayload:   cfg.willPayload = toString(v); break;
    case inWillQoS:       cfg.willQoS = toInt(v); break;
    case inWillRetain:    cfg.willRetain = toBool(v); break;
    case inSubscribeQoS:  cfg.subscribeQoS = toInt(v); break;
    case inPublishQoS:    cfg.publishQoS = toInt(v); break;
    case inPublishRetain: cfg.publishRetain = toBool(v); break;
    case inOptions:       cfg.options = toStrings(v); break;
    default: break;
  }
}

void RsMqttBrokerIntDp::answer4DpId(int index, Variable *varPtr)
{
  if ( index != inReconnect )
    setValue(index, varPtr);
  delete varPtr;
}

void RsMqttBrokerIntDp::hotLink2Internal(int index, Variable *varPtr)
{
  if ( index == inReconnect )
  {
    if ( toBool(varPtr) )
      if ( RsMqttConnection *c = RsMqttConnections::instance().find(broker) )
        c->reconnectRequested = true;
  }
  else
  {
    setValue(index, varPtr);
    if ( isReady() )
      markDirty(broker);
  }
  delete varPtr;
}

void RsMqttBrokerIntDp::dpReady()
{
  if ( getError() )
  {
    ErrHdl::error(ErrClass::PRIO_SEVERE, ErrClass::ERR_PARAM, ErrClass::UNEXPECTEDSTATE, "RsMqttBrokerIntDp", "dpReady",
                  CharString("broker '") + broker.c_str() + "': datapoint " + dpNameOf(broker) +
                  " of type _RsMqttBroker is missing or incomplete" +
                  (RsMqttRedu::isHost2() ? " (redundant host 2: create the _2 datapoints with rsmqttUpgradeToRedundant())" : ""));
  }
  markDirty(broker);
}

void RsMqttBrokerIntDp::handleError(const ErrClass *err)
{
  // e.g. the DP was deleted: called once per element, so act once
  if ( deleted )
    return;
  deleted = true;
  ErrHdl::error(ErrClass::PRIO_WARNING, ErrClass::ERR_PARAM, ErrClass::UNEXPECTEDSTATE, "RsMqttBrokerIntDp", "handleError",
                CharString("broker '") + broker.c_str() + "' disabled: " +
                (err ? err->getErrorText() : CharString("datapoint error")) +
                " (when its datapoint is created again, it is taken over with the next change of the broker list)");
  cfg.active = false;
  markDirty(broker);
}

void RsMqttBrokerIntDp::publishState(int connState, const std::string &lastError, const std::string &serverURI,
                                     const std::string &clientId, unsigned messagesIn, unsigned messagesOut,
                                     bool withCounters)
{
  if ( !isReady() || getError() )
    return;

  auto set = [this](int idx, const Variable &v) {
    if ( isIdValid(idx) )
      Manager::dpSet(getId(idx), v);
  };

  if ( !published || connState != pubState )
    set(outConnState, IntegerVar(connState));
  if ( !published || lastError != pubError )
    set(outLastError, TextVar(lastError.c_str()));
  if ( !published || serverURI != pubUri )
    set(outServerURI, TextVar(serverURI.c_str()));
  if ( !published || clientId != pubClientId )
    set(outClientId, TextVar(clientId.c_str()));
  if ( withCounters )
  {
    // countersPublished, not published: the first publish may come without
    // counters, and the DP may still hold the counts of a previous run
    if ( !countersPublished || messagesIn != pubIn )
      set(outMessagesIn, UIntegerVar(messagesIn));
    if ( !countersPublished || messagesOut != pubOut )
      set(outMessagesOut, UIntegerVar(messagesOut));
    pubIn = messagesIn;
    pubOut = messagesOut;
    countersPublished = true;
  }

  published = true;
  pubState = connState;
  pubError = lastError;
  pubUri = serverURI;
  pubClientId = clientId;
}
