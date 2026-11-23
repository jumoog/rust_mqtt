// RsMqttService.hxx - Manager Service API (MSA) of the driver.
//
// The driver hosts the vRPC service "RsMqtt" as a manager-specific service
// (DRIVER_MAN, -num), so any CTRL manager can subscribe to MQTT topics and
// publish through the driver's broker connections. rsmqttService.ctl wraps
// it in rsmqttTopicConnect() / rsmqttTopicDisconnect() / rsmqttPublish().
//
// Functions (request and response are mappings):
//   Subscribe   {broker, topic, qos?, binary?}               -> {id}
//   Receive     {id, wait?, max?}                            -> {messages, dropped}
//   Renew       {id}                                         -> {ok}
//   Unsubscribe {id}                                         -> {ok}
//   Publish     {broker, topic, payload, qos?, retain?}      -> {ok}
//
// The filters live in the connection's router (crates/rsmqtt-wincc,
// Router::add_service), shared with the addresses; for every message it
// names the subscriptions it is for, once even when overlapping filters
// make the broker send copies, and whether the payload is text. This class
// keeps the queues and answers the calls.
//
// vRPC in 3.21 has unary functions only, so messages reach CTRL by a long
// poll: Receive answers at once when messages are queued, else it is held
// (async handler) until a message arrives, `wait` seconds pass or the call's
// deadline comes close. A subscription that sees no Receive or Renew for
// LEASE is dropped (with a warning counting the messages lost), so a CTRL
// manager that ends without unsubscribing does not leave filters behind; a
// client whose callbacks run long calls Renew in between. The ids hold the
// driver's start time: after a restart the old ids are unknown (NotFound)
// and the client subscribes again.
//
// Everything runs on the driver's main thread (MSA calls come in through
// the manager's dispatch, messages through workProc), so no locking.
#ifndef RSMQTT_SERVICE_HXX
#define RSMQTT_SERVICE_HXX

#include <rsmqtt_wincc.hpp>
#include <vrpc.h>

#include <chrono>
#include <deque>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

class RsMqttConnection;
class Variable;

class RsMqttService final : public vrpc::ServiceBase<RsMqttService>
{
  public:
    RsMqttService();

    /// Registers and starts the service (once; later calls do nothing).
    static void start();
    /// The running service, nullptr before start() or if disabled.
    static RsMqttService *get() { return self.get(); }

    /// A message of a connection: queued for the subscriptions the router named.
    void onMessage(const rsmqtt::wincc::Router::Outcome &out, const std::string &topic, std::string_view payload,
                   int qos, bool retained);
    /// Answers held Receive calls and drops expired subscriptions.
    void workProc();

  private:
    using Clock = std::chrono::steady_clock;

    struct Message
    {
      std::string topic;
      std::string payload;
      int     qos = 0;
      bool    retained = false;
      bool    text = false;   // UTF-8 without NUL bytes: can be a CTRL string
      int64_t timeMs = 0;     // receive time
    };
    using MessagePtr = std::shared_ptr<const Message>;  // one per message, shared by the queues

    /// A Receive that waits for messages.
    struct Held
    {
      std::shared_ptr<vrpc::ServerContext> ctx;
      std::shared_ptr<vrpc::FunctionResponseSender> sender;
      Clock::time_point until;
      unsigned max = 0;
    };

    struct Subscription
    {
      RsMqttConnection *conn = nullptr;  // connections live until the driver exits
      std::string filter;
      bool binary = false;
      std::deque<MessagePtr> queue;
      size_t   queueBytes = 0;
      unsigned dropped = 0;  // messages dropped since the last Receive (queue full)
      Clock::time_point lastSeen;
      std::optional<Held> held;
    };

    vrpc::Status subscribe(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req, Variable *&resp);
    vrpc::Status unsubscribe(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req, Variable *&resp);
    vrpc::Status renew(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req, Variable *&resp);
    vrpc::Status publish(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req, Variable *&resp);
    void receive(std::shared_ptr<vrpc::ServerContext> ctx, const Variable *req,
                 std::shared_ptr<vrpc::FunctionResponseSender> sender);

    /// The subscription of a request's "id" (nullptr if unknown).
    Subscription *find(const Variable *req, uint64_t *id = nullptr);
    /// Sends up to `max` queued messages to `sender`.
    void answer(Subscription &sub, vrpc::FunctionResponseSender &sender, unsigned max);
    /// Answers the held Receive of `sub` with what is queued.
    void finishHeld(Subscription &sub);
    /// Ends a subscription: its held Receive returns empty, the filter is released.
    void release(uint64_t id, Subscription &sub);

    std::unordered_map<uint64_t, Subscription> subs;
    std::vector<uint64_t> ready;  // held Receives that got messages
    Clock::time_point nextScan;   // deadlines, cancellations and leases are checked this often
    std::string idPrefix;         // "<start time in ms, hex>-": ids of this driver run
    uint64_t nextId = 0;

    static std::shared_ptr<RsMqttService> self;
};

#endif
