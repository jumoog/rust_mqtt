// RsMqttConnections.hxx - broker connections of the driver and the
// registry of addresses / subscriptions per connection.
//
// A connection is configured either in the config file ([rsmqtt],
// connection = "<name>") or by a _RsMqttBroker datapoint "_RsMqtt_<name>"
// listed in _RsMqttDriver_<num>.Brokers. Addresses may reference a
// connection before its configuration arrives; it is then a placeholder.
//
// Everything here runs on the driver's main thread (config reading,
// DP hotlinks, addDpPa/clrDpPa, workProc, writeData), so no locking.
#ifndef RSMQTT_CONNECTIONS_HXX
#define RSMQTT_CONNECTIONS_HXX

#include "RsMqttAddress.hxx"
#include "RsMqttJson.hxx"
#include "RsMqttSparkplug.hxx"
#include <rsmqtt.hpp>

#include <chrono>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <vector>

class RsMqttBrokerIntDp;
struct RsMqttBrokerConfig;

class RsMqttConnection
{
  public:
    enum class Source { Placeholder, ConfigFile, Datapoint };

    explicit RsMqttConnection(const std::string &name);

    /// The driver's default options for a connection of this name.
    static rsmqtt::Options defaultOptions(const std::string &name);
    /// "WCCOArsmqtt_<num>_<name>"
    static std::string defaultClientId(const std::string &name);

    // ----- configuration
    std::string     name;
    Source          source = Source::Placeholder;
    rsmqtt::Options options;          // effective rsmqtt options
    int             subscribeQoS = 1; // default QoS of input addresses
    int             publishQoS = 1;   // default QoS of output addresses
    bool            publishRetain = false;
    std::chrono::milliseconds reconnectInterval{10000};  // retry of failed connects
    std::string     clientId;         // as configured; host 2 connects with "<clientId>_2"
    bool            reduClientId = false;  // redu.clientId given: host 2 uses it as it is
    std::string     spbHostId;        // sparkplug.hostId: STATE messages and will of the host application
    std::string     spbReduHostId;    // redu.sparkplug.hostId: the second redundant host's own host id
    bool            willTopicSet = false;  // a will is configured (replaced by the STATE will)

    /// The host id this driver publishes STATE for ("" for none). Two
    /// redundant drivers must not share one: each has its own will and STATE
    /// timestamp, and edge nodes ignore STATE older than the last one. So
    /// host 1 uses sparkplug.hostId and host 2 only redu.sparkplug.hostId
    /// (without it, host 2 reads and writes but is no primary host).
    const std::string &spbStateHost() const;
    int             persistence = -1; // explicit persistence=: -1 not set, 0 none/memory, 1 file
    bool            persistenceDirSet = false;
    std::string     persistenceDir;

    // ----- datapoint configuration
    RsMqttBrokerIntDp *brokerDp = nullptr;  // owned by RsMqttConnections (releaseBrokerDp)
    bool listed = false;                    // in _RsMqttDriver_<num>.Brokers
    bool dirty = false;                     // configuration changed, re-create the client
    std::chrono::steady_clock::time_point dirtySince{};
    bool reconnectRequested = false;

    /// Builds `options` and the driver settings from a broker DP.
    bool applyBrokerConfig(const RsMqttBrokerConfig &cfg, std::string &err);

    /// Remembers settings the driver itself must know about (automaticReconnect,
    /// persistence). Call it for every option set on `options`.
    void noteOption(const std::string &name, const std::string &value);

    /// `options` as the client gets them: file persistence goes to
    /// <project>/data/rsmqtt, or to a relative persistenceDir under <project>/data;
    /// on the second redundant host the client id gets "_2".
    rsmqtt::Options clientOptions() const;

    /// A "redu.<option>" entry: on the second redundant host it sets <option>,
    /// on host 1 it is only checked. Throws rsmqtt::Error for an unknown option.
    void applyReduOption(const std::string &option, const std::string &value);

    /// Options whose value may be encrypted with _DriverSecurity.PublicKey (see RsMqttSecret.hxx).
    static bool isSecretOption(const std::string &name);

    // ----- runtime
    std::unique_ptr<rsmqtt::Client> client;
    bool connected = false;
    bool connecting = false;
    bool everConnected = false;   // since the client was created
    bool autoReconnect = true;    // library reconnects lost connections itself
    bool configError = false;     // configuration / client creation failed
    bool statePushNeeded = false; // a $state address was added
    bool stopped = false;         // HWService::stop() closed the client on purpose
    std::chrono::steady_clock::time_point nextConnectAttempt{};
    std::string lastError;
    std::string serverUri;
    unsigned messagesIn = 0;
    unsigned messagesOut = 0;
    std::chrono::steady_clock::time_point lastStatePublish{};
    bool placeholderWarned = false;

    bool isActive() const;  // should have a running client
    int  connState() const; // RsMqttConnState

    struct Subscription
    {
      int  qos = 0;
      int  flags = 0;
      bool matching = false;                // wildcard / shared filter
      int  sparkplugRefs = 0;               // Sparkplug addresses on this edge node's filter: messages go to `spb`
                                            // (other addresses on the same filter get the raw payload)
      std::map<std::string, int> addresses; // full address -> reference count

      /// How a message is handed to the addresses: built from `addresses`
      /// when they change (planDirty), so that a message needs no address
      /// lookups and one pass over its JSON for all json= / ts= paths.
      struct Delivery
      {
        std::string address;
        bool json = false;
        bool dyn = false;
        int  valueId = -1;  // extractor result of json=<path>
        int  tsId = -1;     // extractor result of ts=<path>
        std::string jsonPath, tsPath;  // for messages
      };
      std::vector<Delivery> plan;
      RsMqttJson::Extractor extractor;
      bool planDirty = true;
    };

    std::map<std::string, Subscription> subscriptions;   // filter -> subscription
    std::set<std::string>               matchingFilters; // subset needing topic matching
    std::map<std::string, int>          stateAddresses;  // "$state" addresses -> ref count

    // ----- Sparkplug B host application (RsMqttSparkplug.hxx)
    RsMqttSparkplug::Host spb;
    mutable int64_t spbStateTs = 0;   // timestamp of this client's STATE messages (online and will)
    std::map<std::string, std::map<std::string, int>> spbIndex;  // spbKey -> input addresses -> ref count
    std::map<std::string, std::string> spbLast;  // address -> last value, sent again as invalid when stale
    static std::string spbKey(const std::string &group, const std::string &node, const std::string &device,
                              const std::string &metric, bool online);
    static std::string spbKey(const RsMqttAddress &a)
    {
      return spbKey(a.spbGroup, a.spbNode, a.spbDevice, a.spbMetric, a.spbOnline);
    }
    std::map<rsmqtt::Token, std::vector<std::string>> subscribeTokens; // pending SUBSCRIBEs

    /// Sends SUBSCRIBE for the given filters (batched). No-op while not connected.
    void subscribe(const std::vector<std::string> &filters);
    /// Sends SUBSCRIBE for all registered filters.
    void subscribeAll();
    void unsubscribe(const std::string &filter);
};

class RsMqttConnections
{
  public:
    static RsMqttConnections &instance();

    /// Finds or creates a connection (a new one is a placeholder).
    RsMqttConnection *add(const std::string &name);
    RsMqttConnection *find(const std::string &name);
    std::vector<std::unique_ptr<RsMqttConnection>> &all() { return list; }

    /// New content of _RsMqttDriver_<num>.Brokers.
    void setBrokerList(const std::vector<std::string> &names);

    /// Disconnects from the broker DP of a broker that left the list, so that
    /// deleting the DP afterwards does not report a hotlink error per element
    /// (~DrvIntDp calls dpDisconnect). Call from the work loop, not from a
    /// DrvIntDp callback. Listing the broker again connects a new object.
    void releaseBrokerDp(RsMqttConnection &conn);

    /// Registers an address of a DPE. Returns true if this full address is
    /// new (the caller creates the HWObject for it).
    bool addAddress(const std::string &fullAddress, const RsMqttAddress &addr, bool input);

    /// Unregisters an address. Returns true if it was the last reference.
    bool removeAddress(const std::string &fullAddress, const RsMqttAddress &addr, bool input);

    /// Parsed form of a registered full address (nullptr if unknown).
    const RsMqttAddress *parsed(const std::string &fullAddress) const;

  private:
    std::vector<std::unique_ptr<RsMqttConnection>> list;
    // the broker DP objects; one whose DP was deleted stays until exit (the
    // event manager already dropped its hotlinks, there is nothing to release)
    std::vector<std::unique_ptr<RsMqttBrokerIntDp>> brokerDps;
    std::map<std::string, std::pair<RsMqttAddress, int>> addresses; // full address -> (parsed, refs)
};

#endif
