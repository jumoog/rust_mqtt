// RsMqttBrokerDp.hxx - broker connections configured by datapoints.
//
// DPT _RsMqttDriver (one DP per driver number, default _RsMqttDriver_<num>)
//   Brokers   dyn_string  names of the brokers this driver serves
//   Version   string      [out] library version
//
// DPT _RsMqttBroker (one DP per broker, name "_RsMqtt_<broker>")
//
// Redundancy: on host 2 both DPs carry "_2" (_RsMqttDriver_3_2,
// _RsMqtt_<broker>_2), see RsMqttRedu.hxx.
//   Config.*   [in]  connection settings, see rsmqtt_types.asc
//   Command.Reconnect   [in]  TRUE: drop and re-open the connection
//   State.*    [out] ConnState, LastError, ServerURI, ClientId, MessagesIn/Out
//
// The broker name is what addresses use: "<broker>:<topic>".
// Changes of Config.* take effect immediately (the client is re-created).
#ifndef RSMQTT_BROKERDP_HXX
#define RSMQTT_BROKERDP_HXX

#include <DrvIntDp.hxx>

#include <string>
#include <vector>

/// Settings read from a _RsMqttBroker DP.
struct RsMqttBrokerConfig
{
  bool        active = false;
  std::string serverURI;
  std::vector<std::string> serverURIs;
  std::string clientId;
  int         mqttVersion = 0;
  int         keepAlive = 0;
  bool        cleanSession = true;
  std::string username;
  std::string password;
  std::string tlsTrustStore;
  std::string tlsKeyStore;
  std::string tlsPrivateKey;
  bool        tlsVerify = false;
  std::string willTopic;
  std::string willPayload;
  int         willQoS = 0;
  bool        willRetain = false;
  int         subscribeQoS = 1;
  int         publishQoS = 1;
  bool        publishRetain = false;
  std::vector<std::string> options;  // "name=value", any rsmqtt option
};

/// Connection state written to State.ConnState
enum RsMqttConnState
{
  RSMQTT_CS_DISABLED     = 0,
  RSMQTT_CS_DISCONNECTED = 1,
  RSMQTT_CS_CONNECTING   = 2,
  RSMQTT_CS_CONNECTED    = 3,
  RSMQTT_CS_FAILURE      = 4,
};

//--------------------------------------------------------------------------------

class RsMqttDriverIntDp : public DrvIntDp
{
  public:
    enum
    {
      inBrokers,
      maxConnect,
      outVersion = maxConnect,
      maxDP
    };

    explicit RsMqttDriverIntDp(const CharString &dpName);

    const CharString &getDpName4Query(int index) override;
    void answer4DpId(int index, Variable *varPtr) override;
    void hotLink2Internal(int index, Variable *varPtr) override;
    void dpReady() override;

  private:
    void handleBrokers(const Variable *varPtr);

    CharString dpName;
    CharString names[maxDP];
};

//--------------------------------------------------------------------------------

class RsMqttBrokerIntDp : public DrvIntDp
{
  public:
    enum
    {
      inActive,
      inServerURI,
      inServerURIs,
      inClientId,
      inMQTTVersion,
      inKeepAlive,
      inCleanSession,
      inUsername,
      inPassword,
      inTlsTrustStore,
      inTlsKeyStore,
      inTlsPrivateKey,
      inTlsVerify,
      inWillTopic,
      inWillPayload,
      inWillQoS,
      inWillRetain,
      inSubscribeQoS,
      inPublishQoS,
      inPublishRetain,
      inOptions,
      inReconnect,
      maxConnect,
      outConnState = maxConnect,
      outLastError,
      outServerURI,
      outClientId,
      outMessagesIn,
      outMessagesOut,
      maxDP
    };

    /// DP name of a broker
    /// "_RsMqtt_<broker>", on the second redundant host "_RsMqtt_<broker>_2"
    static CharString dpNameOf(const std::string &broker);

    explicit RsMqttBrokerIntDp(const std::string &broker);

    const CharString &getDpName4Query(int index) override;
    void answer4DpId(int index, Variable *varPtr) override;
    void hotLink2Internal(int index, Variable *varPtr) override;
    void dpReady() override;
    void handleError(const ErrClass *err) override;

    bool isReady() const { return getState() == Ready; }
    /// The DP was deleted (or became unusable); this object keeps the old DP
    /// ids and is replaced when the broker is listed again.
    bool isDeleted() const { return deleted; }
    const RsMqttBrokerConfig &config() const { return cfg; }

    /// Writes the State.* elements (only values that changed); the message
    /// counters only if withCounters is set.
    void publishState(int connState, const std::string &lastError, const std::string &serverURI,
                      const std::string &clientId, unsigned messagesIn, unsigned messagesOut,
                      bool withCounters);

  private:
    void setValue(int index, const Variable *varPtr);

    std::string broker;
    CharString  names[maxDP];
    RsMqttBrokerConfig cfg;
    bool        deleted = false;

    // last published state, to write only changes
    bool        published = false;
    bool        countersPublished = false;  // separately: counters go out at most once a second
    int         pubState = -1;
    std::string pubError, pubUri, pubClientId;
    unsigned    pubIn = 0, pubOut = 0;
};

#endif
