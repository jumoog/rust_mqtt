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
#include <rsmqtt.hpp>

#include <chrono>
#include <map>
#include <memory>
#include <string>
#include <vector>

class RsMqttBrokerIntDp;
using RsMqttBrokerConfig = rsmqtt::wincc::BrokerConfig;

class RsMqttConnection
{
  public:
    enum class Source { Placeholder, ConfigFile, Datapoint };

    explicit RsMqttConnection(const std::string &name);

    // ----- configuration
    std::string     name;
    Source          source = Source::Placeholder;
    /// rsmqtt options, default QoS / retain, reconnect interval, Sparkplug host ids
    /// (crates/rsmqtt-wincc/src/settings.rs)
    rsmqtt::wincc::Settings settings;

    /// The host id the current client publishes STATE for ("" for none):
    /// sparkplug.hostId on host 1, redu.sparkplug.hostId on host 2 (see
    /// Settings::spb_state_host); taken over by clientOptions().
    const std::string &spbStateHost() const { return spbHost; }

    // ----- datapoint configuration
    RsMqttBrokerIntDp *brokerDp = nullptr;  // owned by RsMqttConnections (releaseBrokerDp)
    bool listed = false;                    // in _RsMqttDriver_<num>.Brokers
    bool dirty = false;                     // configuration changed, re-create the client
    std::chrono::steady_clock::time_point dirtySince{};
    bool reconnectRequested = false;

    /// Builds the settings from a broker DP (decrypting passwords); on failure
    /// they stay as they were.
    bool applyBrokerConfig(const RsMqttBrokerConfig &cfg, std::string &err);

    /// The options the client is created with: file persistence goes to
    /// <project>/data/rsmqtt, or to a relative persistenceDir under <project>/data;
    /// on the second redundant host the client id gets "_2"; a Sparkplug host
    /// application gets its STATE will (new timestamp: settings.stateTs()).
    rsmqtt::Options clientOptions();

  private:
    std::string spbHost;

  public:

    // ----- runtime
    std::unique_ptr<rsmqtt::Client> client;
    bool connected = false;
    bool connecting = false;
    bool everConnected = false;   // since the client was created
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

    /// Addresses, subscriptions, $state addresses and the Sparkplug B host
    /// application (crates/rsmqtt-wincc/src/router.rs)
    rsmqtt::wincc::Router router;
    std::map<rsmqtt::Token, std::vector<std::string>> subscribeTokens; // pending SUBSCRIBEs

    /// Sends SUBSCRIBE for the given filters (batched). No-op while not connected.
    void subscribe(const std::vector<std::string> &filters);
    /// Sends SUBSCRIBE for all registered filters.
    void subscribeAll();
    void unsubscribe(const std::string &filter);
    /// Publishes a message (counted in messagesOut); false and `err` without
    /// a client or if the client refuses it.
    bool publish(const std::string &topic, std::string_view payload, int qos, bool retain, std::string &err);
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
