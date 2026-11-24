// Test of the rsmqtt-wincc C ABI through its C++ wrapper (rsmqtt_wincc.hpp).
// The logic itself is tested in Rust (cargo test -p rsmqtt-wincc); this checks
// that values, strings with NUL bytes, errors and handles cross the boundary.
#include <rsmqtt_wincc.hpp>

#include <cstring>
#include <iostream>
#include <string>

namespace W = rsmqtt::wincc;
namespace J = rsmqtt::wincc::json;
namespace S = rsmqtt::wincc::sparkplug;

static int failures = 0;

#define CHECK(cond)                                                              \
  do {                                                                           \
    if ( !(cond) ) { std::cerr << __LINE__ << ": CHECK failed: " #cond "\n"; failures++; } \
  } while (0)

// protobuf of a Sparkplug payload with one Double metric (name, alias, value)
static std::string varint(uint64_t v)
{
  std::string s;
  while ( v >= 0x80 ) { s += (char) ((v & 0x7F) | 0x80); v >>= 7; }
  s += (char) v;
  return s;
}

static std::string field(int no, const std::string &bytes) { return varint((uint64_t) (no << 3 | 2)) + varint(bytes.size()) + bytes; }

static std::string doubleMetric(const std::string &name, double v, int seq)
{
  std::string m = field(1, name) + varint(2 << 3) + varint(1) + varint(4 << 3) + varint(10) + varint(13 << 3 | 1);
  char raw[8];
  memcpy(raw, &v, 8);
  m.append(raw, 8);
  return varint(1 << 3) + varint(1700000000000ULL) + field(2, m) + varint(3 << 3) + varint((uint64_t) seq);
}

int main()
{
  std::string err;

  // addresses
  W::Address a;
  CHECK(W::Address::parse("plant:$spb/Factory/Line1/PLC7;metric=Motor/speed;qos=1", a, err));
  CHECK(a.connection == "plant" && a.sparkplug && a.spbDevice == "PLC7" && a.spbMetric == "Motor/speed" && a.qos == 1 &&
        a.topic == "spBv1.0/Factory/+/Line1/#");
  CHECK(W::Address::parse("plant:s/1;json=data.temp;ts=data.time", a, err) && a.json && a.jsonPath == "data.temp" && a.hasTs);
  CHECK(!W::Address::parse("plant:a;qos=3", a, err) && err == "qos must be 0, 1 or 2");
  CHECK(W::topicMatches("a/+/c", "a/b/c") && !W::topicMatches("#", "$SYS/x") && W::isWildcardFilter("$share/g/a"));

  // JSON: one pass, results, NUL bytes in strings, errors
  J::Extractor x;
  int temp = x.add("data.temp"), name = x.add("data['n']"), whole = x.add("");
  CHECK(temp == 0 && name == 1 && whole == 2 && x.add("a..b") == -1 && W::lastError().find("empty member") != std::string::npos);
  std::string doc = R"({"data": {"temp": 21.5, "n": "a\u0000b"}})";
  x.run(doc);
  J::Kind k;
  std::string_view v;
  CHECK(x.result(temp, k, v) == J::Found::Yes && k == J::Kind::Number && v == "21.5");
  CHECK(x.result(name, k, v) == J::Found::Yes && k == J::Kind::String && v == std::string_view("a\0b", 3));
  CHECK(x.result(whole, k, v) == J::Found::Yes && k == J::Kind::Object);
  x.run("not json");
  CHECK(x.result(temp, k, v) == J::Found::NotJson && !x.error().empty());

  J::Value jv;
  CHECK(J::get(R"({"online": false})", "online", jv, err) == J::Found::Yes && jv.text == "false");
  CHECK(J::quote(std::string("a\"\0", 3)) == "\"a\\\"\\u0000\"");
  CHECK(J::pathString("$.a['b.c']") == "a[\"b.c\"]");
  std::string built;
  CHECK(J::build({{"data.value", "12.5"}, {"data.time", "\"T\""}}, built, err) && built == R"({"data":{"value":12.5,"time":"T"}})");
  CHECK(!J::build({{"a", "1"}, {"a.b", "2"}}, built, err) && err.find("collides") != std::string::npos);
  long long ms = 0;
  CHECK(J::parseTime(J::Kind::String, "2023-11-14T22:13:20.123Z", ms) && ms == 1700000000123LL);
  CHECK(J::isoTime(1700000000123LL) == "2023-11-14T22:13:20.123Z");
  std::vector<std::pair<std::string, bool>> items;
  CHECK(J::dynItems("[1, \"x\"]", items, err) && items.size() == 2 && items[1].first == "x" && items[1].second);
  CHECK(!J::dynItems("[1, null]", items, err) && err.find("null") != std::string::npos);

  // redundancy names
  CHECK(W::redu::forHost("_RsMqtt_plant", true) == "_RsMqtt_plant_2" && W::redu::endsWithSuffix("x_2"));
  std::string opt;
  CHECK(W::redu::splitReduOption("REDU.clientId", opt) && opt == "clientId" && !W::redu::splitReduOption("reduce", opt));
  CHECK(W::redu::clientIdForHost("c", true, false) == "c_2");

  // Sparkplug host
  S::Host h;
  std::vector<S::Update> u;
  std::vector<S::Message> pub;
  std::vector<std::string> warn;
  h.onMessage("spBv1.0/G/NBIRTH/N", doubleMetric("temp", 21.5, 0), 1000, u, pub, warn);
  CHECK(u.size() == 2 && u[0].kind == S::Update::Kind::Value && u[0].metric == "temp" && u[0].text == "21.5" &&
        u[0].hasTime && u[0].timeMs == 1700000000000LL && u[1].kind == S::Update::Kind::Online && u[1].online);
  CHECK(h.isOnline("G", "N", "") && h.dataType("G", "N", "", "temp") == 10);
  u.clear();
  h.onMessage("spBv1.0/G/NDATA/N", doubleMetric("temp", 22, 5), 1000, u, pub, warn);  // seq gap
  CHECK(u.size() == 1 && pub.size() == 1 && pub[0].topic == "spBv1.0/G/NCMD/N" && warn.size() == 2);
  S::Message cmd;
  CHECK(h.command("G", "N", "", "temp", "19.5", 1000, cmd, err) && cmd.topic == "spBv1.0/G/NCMD/N" && !cmd.payload.empty());
  CHECK(!h.command("G", "N", "", "missing", "1", 1000, cmd, err) && err.find("not in the birth") != std::string::npos);
  u.clear();
  h.onConnectionLost(u);
  CHECK(u.size() == 2 && u[0].kind == S::Update::Kind::Stale && !h.isOnline("G", "N", ""));
  CHECK(S::stateTopic("h") == "spBv1.0/STATE/h" && S::statePayload(true, 5) == R"({"online":true,"timestamp":5})");
  CHECK(S::nodeFilter("G", "N") == "spBv1.0/G/+/N/#" && S::validId("N") && !S::validId("a/b"));

  // connection settings: broker config, reveal callback, errors, client options
  {
    W::Settings st("plant", 3);
    CHECK(st.subscribeQoS() == 1 && st.publishQoS() == 1 && !st.publishRetain() && st.autoReconnect() &&
          st.reconnectInterval() == std::chrono::seconds(10));
    W::BrokerConfig cfg;
    cfg.serverURI = "tcp://localhost:1883";
    cfg.password = "secret";
    cfg.subscribeQoS = 2;
    cfg.options = {"automaticReconnect=0", "sparkplug.hostId=scada", "redu.clientId=b"};
    std::vector<std::string> seen;
    W::Settings::Reveal reveal = [&seen](const std::string &stored, std::string &plain, std::string &e) {
      seen.push_back(stored);
      if ( stored == "broken" ) { e = "decryption failed"; return false; }
      plain = std::string("p\0w", 3);  // NUL bytes survive
      return true;
    };
    CHECK(st.applyBrokerConfig(cfg, false, reveal, err) && seen.size() == 1 && seen[0] == "secret");
    CHECK(st.subscribeQoS() == 2 && !st.autoReconnect() && st.spbStateHost(false) == "scada" && st.spbStateHost(true).empty());
    cfg.password = "broken";
    CHECK(!st.applyBrokerConfig(cfg, false, reveal, err) && err == "Config.Password: decryption failed");
    cfg.password.clear();
    cfg.options = {"nonsense"};
    CHECK(!st.applyBrokerConfig(cfg, false, nullptr, err) && err == "Config.Options entry 'nonsense' is not 'name=value'");
    CHECK(st.subscribeQoS() == 2);  // unchanged on failure

    std::vector<std::string> warnings;
    st.setOption("will.topic", "lwt", err);
    rsmqtt::Options o = st.clientOptions(false, "", 1234, warnings);
    CHECK(warnings.size() == 1 && warnings[0].find("STATE will") != std::string::npos && st.stateTs() == 1234);
    rsmqtt::Client c(st.clientOptions(true, "", 1, warnings));
    CHECK(c.clientId() == "WCCOArsmqtt_3_plant_2");  // redu.clientId was applied for host 1: only checked
    CHECK(!st.setOption("will.qos", "3", err) && err.find("QoS must be 0, 1 or 2") != std::string::npos);
    CHECK(!st.applyReduOption("bogus", "1", false, err) && err == "unknown option 'redu.bogus'");
    CHECK(!st.setSpbHostId("a/b", err) && W::Settings::isSecretOption("Pass_word") && !W::Settings::isSecretOption("user"));
  }

  // message router: registry, deliveries with and without a value, $state, Sparkplug
  {
    using R = W::Router;
    R r;
    std::string filter;
    CHECK(r.add("p:s/+", false, true, 1, filter, err) == R::Added::Subscribe && filter == "s/+");
    CHECK(r.add("p:s/+;json=v;ts=t", true, true, 1, filter, err) == R::Added::Nothing);  // same filter: plan only
    CHECK(r.add("p:$state", false, true, 1, filter, err) == R::Added::State);
    CHECK(r.add("p:a;qos=9", false, true, 1, filter, err) == R::Added::Error && err == "qos must be 0, 1 or 2");
    CHECK(r.add("p:$spb/G/N;online", false, true, 1, filter, err) == R::Added::Subscribe && filter == "spBv1.0/G/+/N/#");
    int qos = -1, flags = -1;
    CHECK(r.filter("s/+", qos, flags) && qos == 1 && flags == 0 && !r.filter("x", qos, flags));
    CHECK((r.filters() == std::vector<std::string>{"s/+", "spBv1.0/G/+/N/#"}) && r.hasAddresses());

    const R::Outcome &o = r.message("s/1", std::string(R"({"v":"a\u0000b","t":1700000000000})"), 0, "", 0);
    CHECK(o.deliveries.size() == 2 && std::string(o.deliveries[0].address) == "p:s/+" && !o.deliveries[0].hasValue);
    CHECK(o.deliveries[1].hasValue && o.deliveries[1].value == std::string("\"a\\u0000b\"") && o.deliveries[1].hasTime &&
          o.deliveries[1].timeMs == 1700000000000LL && o.deliveries[1].periphTime);
    const R::Outcome &st = r.state(true);
    CHECK(st.deliveries.size() == 1 && st.deliveries[0].value == "1" && st.deliveries[0].validity == RSMQTT_WINCC_VALIDITY_KEEP);
    const R::Outcome &c = r.connected("scada", 7);
    CHECK(c.subscribe.size() == 1 && std::string(c.subscribe[0].first) == "spBv1.0/STATE/scada" && c.publish.size() == 1 &&
          c.publish[0].payload == R"({"online":true,"timestamp":7})" && c.publish[0].retain && c.deliveries.size() == 1 &&
          c.deliveries[0].value == "0" && c.deliveries[0].validity == RSMQTT_WINCC_VALIDITY_VALID);
    const R::Outcome &w = r.message("s/2", "{}", 0, "", 0);
    CHECK(w.warnings.size() == 1 && !w.warnings[0].first && std::string(w.warnings[0].second).find("no value at v") != std::string::npos);
    R::Write wr;
    CHECK(!r.write("p:$spb/G/N;online", "1", 1007, 0, 0, 1, false, wr, err) && err == "address 'p:$spb/G/N;online': cannot be written");
    CHECK(r.add("p:out;json=v", false, false, 1, filter, err) == R::Added::Nothing);
    CHECK(r.write("p:out;json=v", std::string("a\0b", 3), 1001, 0, 0, 2, true, wr, err) && wr.topic == "out" &&
          wr.payload == R"({"v":"a\u0000b"})" && wr.qos == 2 && wr.retain && !wr.deferred);
    // merge: written values wait for flush(), one document per topic
    CHECK(r.add("p:m;json=a;merge", false, false, 1, filter, err) == R::Added::Nothing);
    CHECK(r.add("p:m;json=b;merge", false, false, 1, filter, err) == R::Added::Nothing);
    CHECK(r.add("p:m;json=a.x;merge", false, false, 1, filter, err) == R::Added::Error && err.find("collide") != std::string::npos);
    CHECK(r.write("p:m;json=a;merge", "1", 1006, 0, 0, 1, false, wr, err) && wr.deferred && wr.topic.empty());
    CHECK(r.write("p:m;json=b;merge", "x", 1001, 0, 0, 1, false, wr, err) && wr.deferred);
    const R::Outcome &f = r.flush(1, true);
    CHECK(f.publish.size() == 1 && std::string(f.publish[0].topic) == "m" && f.publish[0].payload == R"({"a":1,"b":"x"})" &&
          f.publish[0].qos == 1 && f.publish[0].retain);
    CHECK(r.flush(1, true).publish.empty());
    CHECK(!r.remove("p:s/+", true, filter) && r.remove("p:s/+;json=v;ts=t", true, filter) && filter == "s/+");
  }

  // values: numbers and bool from payload text
  {
    rsmqtt_wincc_number n{};
    CHECK(W::values::parseNumber(1002, " \"12.0\" ", false, n, err) && n.i == 12);
    CHECK(W::values::parseNumber(1005, "18446744073709551615", false, n, err) && n.u == UINT64_MAX);
    CHECK(W::values::parseNumber(1006, "true", false, n, err) && n.f == 1.0);
    CHECK(W::values::parseNumber(1007, "off", false, n, err) && n.b == 0);
    CHECK(!W::values::parseNumber(1002, std::string("1\0", 2), false, n, err) && err.find("cannot convert") == 0);
  }

  if ( failures )
  {
    std::cerr << failures << " check(s) failed\n";
    return 1;
  }
  std::cout << "wincc_test: all checks passed\n";
  return 0;
}
