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

  if ( failures )
  {
    std::cerr << failures << " check(s) failed\n";
    return 1;
  }
  std::cout << "wincc_test: all checks passed\n";
  return 0;
}
