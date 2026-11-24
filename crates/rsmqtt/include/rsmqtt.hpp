// rsmqtt.hpp - header-only C++17 wrapper of the rsmqtt C API.
//
// RAII handles, std::string_view in, exceptions (rsmqtt::Error) on failure:
//
//   rsmqtt::Options opts;
//   opts.set("serverURI", "tcp://localhost:1883")
//       .set("clientId", "winccoa-1")
//       .set("automaticReconnect", true);
//   rsmqtt::Client cli(opts);
//   cli.connect();
//   cli.subscribe("plant/#", 1);
//   while (auto ev = cli.poll()) {
//     if (ev->type() == rsmqtt::EventType::Message)
//       std::cout << ev->topic() << " = " << ev->payload() << "\n";
//   }
//
// The C API stays the binary boundary (Rust cannot export a C++ ABI); this
// header only adds C++ ergonomics on top, so it compiles with any C++17
// compiler and needs no generated code.
#ifndef RSMQTT_HPP
#define RSMQTT_HPP

#include "rsmqtt.h"

#include <chrono>
#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace rsmqtt {

// ---------------------------------------------------------------- errors

class Error : public std::runtime_error {
public:
  Error(int rc, const std::string &what) : std::runtime_error(what), rc_(rc) {}
  int code() const noexcept { return rc_; }

private:
  int rc_;
};

inline std::string errorText(int rc) {
  const char *last = rsmqtt_last_error();
  if (last && *last)
    return last;
  const char *s = rsmqtt_strerror(rc);
  return s ? s : "unknown error";
}

inline void check(int rc) {
  if (rc < 0)
    throw Error(rc, errorText(rc));
}

inline const char *strerror(int rc) { return rsmqtt_strerror(rc); }
inline const char *reasonCodeString(int reason) { return rsmqtt_reason_code_string(reason); }
inline const char *version() { return rsmqtt_version(); }

using Token = rsmqtt_token;

enum class EventType : int {
  None = 0,
  Connected = RSMQTT_EVENT_CONNECTED,
  ConnectionLost = RSMQTT_EVENT_CONNECTION_LOST,
  Disconnected = RSMQTT_EVENT_DISCONNECTED,
  Message = RSMQTT_EVENT_MESSAGE,
  Complete = RSMQTT_EVENT_COMPLETE,
};

enum class Op : int {
  None = 0,
  Connect = RSMQTT_OP_CONNECT,
  Publish = RSMQTT_OP_PUBLISH,
  Subscribe = RSMQTT_OP_SUBSCRIBE,
  Unsubscribe = RSMQTT_OP_UNSUBSCRIBE,
  Disconnect = RSMQTT_OP_DISCONNECT,
};

// ------------------------------------------------------------ properties

struct Property {
  int id = 0;
  std::uint32_t intValue = 0;
  std::string_view data;  // string / binary / user property value
  std::string_view name;  // user property name
};

// Read access, shared by owned properties and borrowed event properties.
class PropertiesView {
public:
  explicit PropertiesView(const rsmqtt_properties *p = nullptr) : p_(p) {}

  std::size_t size() const { return rsmqtt_properties_count(p_); }
  bool empty() const { return size() == 0; }

  Property at(std::size_t index) const {
    rsmqtt_property raw{};
    check(rsmqtt_properties_get(p_, index, &raw));
    Property prop;
    prop.id = raw.id;
    prop.intValue = raw.int_value;
    if (raw.data)
      prop.data = std::string_view(raw.data, raw.data_len);
    if (raw.name)
      prop.name = std::string_view(raw.name, raw.name_len);
    return prop;
  }

  std::vector<Property> all() const {
    std::vector<Property> v;
    for (std::size_t i = 0, n = size(); i < n; ++i)
      v.push_back(at(i));
    return v;
  }

  // First property with this id.
  std::optional<Property> find(int id) const {
    for (std::size_t i = 0, n = size(); i < n; ++i) {
      Property p = at(i);
      if (p.id == id)
        return p;
    }
    return std::nullopt;
  }

  std::optional<std::string> user(const std::string &name) const {
    for (std::size_t i = 0, n = size(); i < n; ++i) {
      Property p = at(i);
      if (p.id == RSMQTT_PROP_USER_PROPERTY && p.name == name)
        return std::string(p.data);
    }
    return std::nullopt;
  }

  const rsmqtt_properties *get() const { return p_; }

protected:
  const rsmqtt_properties *p_;
};

class Properties : public PropertiesView {
public:
  Properties() : PropertiesView(rsmqtt_properties_new()) {}
  Properties(const Properties &o) : PropertiesView(rsmqtt_properties_clone(o.p_)) {}
  explicit Properties(const PropertiesView &v) : PropertiesView(rsmqtt_properties_clone(v.get())) {}
  Properties(Properties &&o) noexcept : PropertiesView(o.p_) { o.p_ = nullptr; }
  Properties &operator=(Properties o) noexcept {
    std::swap(p_, o.p_);
    return *this;
  }
  ~Properties() { rsmqtt_properties_free(mut()); }

  Properties &add(int id, std::uint32_t value) {
    check(rsmqtt_properties_add_int(mut(), id, value));
    return *this;
  }
  Properties &add(int id, const std::string &value) {
    check(rsmqtt_properties_add_string(mut(), id, value.c_str()));
    return *this;
  }
  Properties &add(int id, const char *value) { return add(id, std::string(value)); }
  Properties &addBinary(int id, std::string_view data) {
    check(rsmqtt_properties_add_binary(mut(), id, data.data(), data.size()));
    return *this;
  }
  Properties &addUser(const std::string &name, const std::string &value) {
    check(rsmqtt_properties_add_user(mut(), name.c_str(), value.c_str()));
    return *this;
  }
  void clear() { rsmqtt_properties_clear(mut()); }

private:
  rsmqtt_properties *mut() { return const_cast<rsmqtt_properties *>(p_); }
};

// --------------------------------------------------------------- options

class Options {
public:
  Options() : o_(rsmqtt_options_new()) {}
  /// Takes over a handle created elsewhere (e.g. by rsmqtt-wincc).
  static Options adopt(rsmqtt_options *owned) { return Options(owned, 0); }
  Options(const Options &o) : o_(rsmqtt_options_clone(o.o_)) {}
  Options(Options &&o) noexcept : o_(o.o_) { o.o_ = nullptr; }
  Options &operator=(Options o) noexcept {
    std::swap(o_, o.o_);
    return *this;
  }
  ~Options() { rsmqtt_options_free(o_); }

  Options &set(const std::string &name, const std::string &value) {
    check(rsmqtt_options_set(o_, name.c_str(), value.c_str()));
    return *this;
  }
  Options &set(const std::string &name, const char *value) { return set(name, std::string(value)); }
  Options &set(const std::string &name, bool value) { return set(name, value ? "1" : "0"); }
  Options &set(const std::string &name, int value) { return set(name, std::to_string(value)); }
  Options &set(const std::string &name, long long value) { return set(name, std::to_string(value)); }
  Options &set(const std::string &name, double value) { return set(name, std::to_string(value)); }
  template <class Rep, class Period>
  Options &set(const std::string &name, std::chrono::duration<Rep, Period> d) {
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(d).count();
    return set(name, std::to_string(ms) + "ms");
  }
  Options &setBinary(const std::string &name, std::string_view data) {
    check(rsmqtt_options_set_binary(o_, name.c_str(), data.data(), data.size()));
    return *this;
  }
  Options &connectProperties(const PropertiesView &p) {
    check(rsmqtt_options_set_properties(o_, RSMQTT_PROPS_CONNECT, p.get()));
    return *this;
  }
  Options &willProperties(const PropertiesView &p) {
    check(rsmqtt_options_set_properties(o_, RSMQTT_PROPS_WILL, p.get()));
    return *this;
  }

  static bool isKnown(const std::string &name) { return rsmqtt_option_is_known(name.c_str()) != 0; }

  const rsmqtt_options *get() const { return o_; }

private:
  Options(rsmqtt_options *owned, int) : o_(owned) {
    if (!o_)
      throw Error(RSMQTT_NULL_PARAMETER, "no options");
  }
  rsmqtt_options *o_;
};

// ----------------------------------------------------------------- event

class Event {
public:
  explicit Event(rsmqtt_event *e) : e_(e) {}
  Event(Event &&o) noexcept : e_(o.e_) { o.e_ = nullptr; }
  Event &operator=(Event &&o) noexcept {
    std::swap(e_, o.e_);
    return *this;
  }
  Event(const Event &) = delete;
  Event &operator=(const Event &) = delete;
  ~Event() { rsmqtt_event_free(e_); }

  EventType type() const { return static_cast<EventType>(rsmqtt_event_type(e_)); }
  Op op() const { return static_cast<Op>(rsmqtt_event_op(e_)); }
  Token token() const { return rsmqtt_event_token(e_); }
  int rc() const { return rsmqtt_event_rc(e_); }
  bool ok() const { return rc() == RSMQTT_SUCCESS; }
  int reasonCode() const { return rsmqtt_event_reason_code(e_); }
  std::vector<int> reasonCodes() const {
    std::vector<int> v;
    for (std::size_t i = 0, n = rsmqtt_event_reason_code_count(e_); i < n; ++i)
      v.push_back(rsmqtt_event_reason_code_at(e_, i));
    return v;
  }
  bool sessionPresent() const { return rsmqtt_event_session_present(e_) != 0; }
  int mqttVersion() const { return rsmqtt_event_mqtt_version(e_); }
  std::string_view serverUri() const { return sv(rsmqtt_event_server_uri(e_)); }
  std::string_view text() const { return sv(rsmqtt_event_text(e_)); }

  std::string_view topic() const {
    std::size_t len = 0;
    const char *t = rsmqtt_event_topic(e_, &len);
    return t ? std::string_view(t, len) : std::string_view();
  }
  std::string_view payload() const {
    std::size_t len = 0;
    const void *p = rsmqtt_event_payload(e_, &len);
    return p ? std::string_view(static_cast<const char *>(p), len) : std::string_view();
  }
  int qos() const { return rsmqtt_event_qos(e_); }
  bool retained() const { return rsmqtt_event_retained(e_) != 0; }
  bool dup() const { return rsmqtt_event_dup(e_) != 0; }
  int msgid() const { return rsmqtt_event_msgid(e_); }
  PropertiesView properties() const { return PropertiesView(rsmqtt_event_properties(e_)); }

  const rsmqtt_event *get() const { return e_; }

private:
  static std::string_view sv(const char *s) { return s ? std::string_view(s) : std::string_view(); }
  rsmqtt_event *e_;
};

// ---------------------------------------------------------------- client

class Client {
public:
  explicit Client(const Options &opts) { check(rsmqtt_client_create(opts.get(), &c_)); }
  Client(Client &&o) noexcept : c_(o.c_) { o.c_ = nullptr; }
  Client &operator=(Client &&o) noexcept {
    std::swap(c_, o.c_);
    return *this;
  }
  Client(const Client &) = delete;
  Client &operator=(const Client &) = delete;
  ~Client() { rsmqtt_client_destroy(c_); }

  // Options for the next connect() (e.g. new credentials).
  void setOptions(const Options &opts) { check(rsmqtt_client_set_options(c_, opts.get())); }
  // cb is called from a library thread when the event queue becomes non-empty.
  void setNotify(rsmqtt_notify_fn cb, void *ctx) { check(rsmqtt_set_event_notify(c_, cb, ctx)); }

  Token connect() {
    Token t = 0;
    check(rsmqtt_connect(c_, &t));
    return t;
  }
  Token reconnect() {
    Token t = 0;
    check(rsmqtt_reconnect(c_, &t));
    return t;
  }
  Token disconnect(std::chrono::milliseconds timeout = std::chrono::seconds(10), int reasonCode = -1,
                   bool publishWill = false, const PropertiesView *props = nullptr) {
    Token t = 0;
    check(rsmqtt_disconnect(c_, toMs(timeout), reasonCode, publishWill ? RSMQTT_DISCONNECT_PUBLISH_WILL : 0,
                            props ? props->get() : nullptr, &t));
    return t;
  }
  bool isConnected() const { return rsmqtt_is_connected(c_) != 0; }

  Token publish(const std::string &topic, std::string_view payload, int qos = 1, bool retained = false,
                const PropertiesView *props = nullptr) {
    Token t = 0;
    check(rsmqtt_publish(c_, topic.c_str(), payload.data(), payload.size(), qos, retained ? 1 : 0,
                         props ? props->get() : nullptr, &t));
    return t;
  }

  Token subscribe(const std::string &filter, int qos = 1, int flags = 0, const PropertiesView *props = nullptr) {
    Token t = 0;
    check(rsmqtt_subscribe(c_, filter.c_str(), qos, flags, props ? props->get() : nullptr, &t));
    return t;
  }
  Token subscribe(const std::vector<std::string> &filters, const std::vector<int> &qos,
                  const std::vector<int> &flags = {}, const PropertiesView *props = nullptr) {
    if (filters.size() != qos.size() || (!flags.empty() && flags.size() != filters.size()))
      throw Error(RSMQTT_INVALID_ARGUMENT, "filters, qos and flags must have the same length");
    std::vector<const char *> f;
    for (const auto &s : filters)
      f.push_back(s.c_str());
    Token t = 0;
    check(rsmqtt_subscribe_many(c_, f.size(), f.data(), qos.data(), flags.empty() ? nullptr : flags.data(),
                                props ? props->get() : nullptr, &t));
    return t;
  }

  Token unsubscribe(const std::string &filter, const PropertiesView *props = nullptr) {
    Token t = 0;
    check(rsmqtt_unsubscribe(c_, filter.c_str(), props ? props->get() : nullptr, &t));
    return t;
  }
  Token unsubscribe(const std::vector<std::string> &filters, const PropertiesView *props = nullptr) {
    std::vector<const char *> f;
    for (const auto &s : filters)
      f.push_back(s.c_str());
    Token t = 0;
    check(rsmqtt_unsubscribe_many(c_, f.size(), f.data(), props ? props->get() : nullptr, &t));
    return t;
  }

  // Returns the operation's result code (RSMQTT_TIMEOUT if not done in time).
  int waitForCompletion(Token token, std::chrono::milliseconds timeout) {
    return rsmqtt_wait_for_completion(c_, token, toMs(timeout));
  }
  // Like waitForCompletion, but throws on any failure.
  void wait(Token token, std::chrono::milliseconds timeout = std::chrono::seconds(30)) {
    check(waitForCompletion(token, timeout));
  }

  // Next event; timeout 0 = don't wait, negative = wait forever.
  std::optional<Event> poll(std::chrono::milliseconds timeout = std::chrono::milliseconds(0)) {
    rsmqtt_event *e = nullptr;
    int rc = rsmqtt_poll_event(c_, toMs(timeout), &e);
    check(rc);
    if (rc == 1 && e)
      return Event(e);
    return std::nullopt;
  }

  std::vector<Token> pendingTokens() {
    rsmqtt_token *toks = nullptr;
    std::size_t n = 0;
    check(rsmqtt_get_pending_tokens(c_, &toks, &n));
    std::vector<Token> v(toks, toks + n);
    rsmqtt_free_tokens(toks, n);
    return v;
  }

  std::size_t queuedEvents() const { return rsmqtt_queued_events(c_); }
  std::uint64_t droppedEvents() const { return rsmqtt_dropped_events(c_); }

  std::string clientId() const {
    ptrdiff_t n = rsmqtt_client_id(c_, nullptr, 0);
    if (n <= 0)
      return {};
    std::string s(static_cast<std::size_t>(n) + 1, '\0');
    rsmqtt_client_id(c_, s.data(), s.size());
    s.resize(static_cast<std::size_t>(n));
    return s;
  }

  rsmqtt_client *get() const { return c_; }

private:
  template <class D> static int toMs(D d) {
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(d).count();
    if (ms > 0x7fffffff)
      return 0x7fffffff;
    return static_cast<int>(ms);
  }
  rsmqtt_client *c_ = nullptr;
};

} // namespace rsmqtt

#endif // RSMQTT_HPP
