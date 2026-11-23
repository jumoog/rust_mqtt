// Unit test of the driver's RsMqttAddress (parser and topic matcher).
#include "RsMqttAddress.hxx"
#include "RsMqttRedu.hxx"

#include <iostream>

static int failures = 0;

#define CHECK(cond)                                                              \
  do {                                                                           \
    if ( !(cond) ) { std::cerr << __LINE__ << ": CHECK failed: " #cond "\n"; failures++; } \
  } while (0)

int main()
{
  RsMqttAddress a;
  std::string err;

  CHECK(RsMqttAddress::parse("plant:line1/temp", a, err));
  CHECK(a.connection == "plant" && a.topic == "line1/temp" && a.qos == -1 && a.retain == -1);
  CHECK(a.key() == "plant:line1/temp");

  CHECK(RsMqttAddress::parse(" plant : a/b;qos=2;retain;nolocal;rh=1 ", a, err));
  CHECK(a.connection == "plant" && a.topic == " a/b");  // topic is taken verbatim
  CHECK(a.qos == 2 && a.retain == 1 && a.subFlags == (0x04 | 0x10));

  CHECK(RsMqttAddress::parse("plant:a;retain=0;rap", a, err));
  CHECK(a.retain == 0 && a.subFlags == 0x08);

  CHECK(RsMqttAddress::parse("plant:$state", a, err) && a.isState);
  CHECK(RsMqttAddress::parse("plant:a/+/c", a, err) && a.needsMatching());
  CHECK(RsMqttAddress::parse("plant:$share/g/a/b", a, err) && a.needsMatching());
  CHECK(RsMqttAddress::parse("plant:a/b", a, err) && !a.needsMatching());

  CHECK(!RsMqttAddress::parse("no-colon", a, err));
  CHECK(!RsMqttAddress::parse(":topic", a, err));
  CHECK(!RsMqttAddress::parse("plant:", a, err));
  CHECK(!RsMqttAddress::parse("plant:a;qos=3", a, err));
  CHECK(!RsMqttAddress::parse("plant:a;bogus", a, err));
  CHECK(!RsMqttAddress::parse("plant:a/#/b", a, err));
  CHECK(!RsMqttAddress::parse("plant:a/b+", a, err));

  using M = RsMqttAddress;
  CHECK(M::topicMatches("a/b", "a/b"));
  CHECK(!M::topicMatches("a/b", "a/c"));
  CHECK(M::topicMatches("a/+", "a/b"));
  CHECK(!M::topicMatches("a/+", "a/b/c"));
  CHECK(!M::topicMatches("a/+", "a"));
  CHECK(M::topicMatches("a/+", "a/"));
  CHECK(M::topicMatches("a/#", "a"));
  CHECK(M::topicMatches("a/#", "a/b/c"));
  CHECK(M::topicMatches("#", "a/b"));
  CHECK(M::topicMatches("+/+/c", "a/b/c"));
  CHECK(!M::topicMatches("#", "$SYS/broker"));
  CHECK(!M::topicMatches("+/broker", "$SYS/broker"));
  CHECK(M::topicMatches("$SYS/#", "$SYS/broker"));
  CHECK(M::topicMatches("$share/grp/a/+", "a/b"));
  CHECK(!M::topicMatches("$share/grp", "a"));
  CHECK(!M::topicMatches("a/b/c", "a/b"));

  // ---------------------------------------------------------------- JSON
  namespace J = RsMqttJson;
  J::Path p;
  J::Value v;
  std::string out;

  CHECK(RsMqttAddress::parse("plant:s/1;json=data.temp;ts=data.time", a, err));
  CHECK(a.json && a.hasTs && J::toString(a.jsonPath) == "data.temp" && J::toString(a.tsPath) == "data.time");
  CHECK(RsMqttAddress::parse("plant:s/1;json", a, err) && a.json && a.jsonPath.empty());
  CHECK(!RsMqttAddress::parse("plant:s/1;ts=time", a, err));           // ts without json
  CHECK(!RsMqttAddress::parse("plant:s/1;json=a;ts=a.b", a, err));     // collide
  CHECK(!RsMqttAddress::parse("plant:s/1;json;ts=t", a, err));         // root value + ts collide
  CHECK(!RsMqttAddress::parse("plant:$state;json", a, err));
  CHECK(!RsMqttAddress::parse("plant:s;json=a..b", a, err));

  CHECK(J::parsePath("values[2].x", p, err) && p.size() == 3 && p[1].isIndex && p[1].index == 2);
  CHECK(J::parsePath("$.a[\"b.c\"]['d']", p, err) && p.size() == 3 && p[1].key == "b.c" && p[2].key == "d");
  CHECK(J::parsePath("$value", p, err) && p.size() == 1 && p[0].key == "$value");
  CHECK(J::parsePath("$", p, err) && p.empty());
  CHECK(J::parsePath("[0]", p, err) && p.size() == 1 && p[0].isIndex);
  CHECK(!J::parsePath("a[x]", p, err));
  CHECK(!J::parsePath("a.", p, err));
  CHECK(!J::parsePath("a]", p, err));

  const char *doc = R"( {"data": {"temp": 21.5, "on": true, "name": "Pump \"1\" é😀",
                         "n": null, "list": [1, {"x": [10, 20]}], "obj": {"a": 1} }, "t": 1700000000} )";
  J::parsePath("data.temp", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.kind == J::Kind::Number && v.text == "21.5");
  J::parsePath("data.on", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.kind == J::Kind::Bool && v.text == "true");
  J::parsePath("data.name", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.kind == J::Kind::String &&
        v.text == "Pump \"1\" \xC3\xA9\xF0\x9F\x98\x80");
  J::parsePath("data.n", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.kind == J::Kind::Null);
  J::parsePath("data.list[1].x[1]", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.text == "20");
  J::parsePath("data.obj", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Yes && v.kind == J::Kind::Object && v.text == "{\"a\": 1}");
  J::parsePath("data.missing", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Missing);
  J::parsePath("data.list[5]", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Missing);
  J::parsePath("data.temp.x", p, err);
  CHECK(J::get(doc, p, v, err) == J::Found::Missing);
  J::parsePath("a", p, err);
  CHECK(J::get("hello", p, v, err) == J::Found::NotJson);
  CHECK(J::get("{\"a\":", p, v, err) == J::Found::NotJson);
  p.clear();
  CHECK(J::get(" 42 ", p, v, err) == J::Found::Yes && v.text == "42");
  CHECK(J::get("\"x\"", p, v, err) == J::Found::Yes && v.kind == J::Kind::String && v.text == "x");
  CHECK(J::get("42 x", p, v, err) == J::Found::NotJson);
  CHECK(J::get("01", p, v, err) == J::Found::NotJson);

  CHECK(J::quote("a\"b\\c\n\x01") == "\"a\\\"b\\\\c\\n\\u0001\"");
  CHECK(J::quote("\xC3\xA9") == "\"\xC3\xA9\"");
  CHECK(J::quote("\xFF") == "\"\xEF\xBF\xBD\"");

  J::Path pv, pt, pr;
  J::parsePath("data.value", pv, err);
  J::parsePath("data.ts", pt, err);
  CHECK(J::build({{&pv, "30"}, {&pt, "\"T\""}}, out, err) && out == "{\"data\":{\"value\":30,\"ts\":\"T\"}}");
  J::parsePath("values[2]", pv, err);
  CHECK(J::build({{&pv, "true"}}, out, err) && out == "{\"values\":[null,null,true]}");
  CHECK(J::build({{&pr, "\"x\""}}, out, err) && out == "\"x\"");
  J::parsePath("a", pv, err);
  J::parsePath("a.b", pt, err);
  CHECK(!J::build({{&pv, "1"}, {&pt, "2"}}, out, err));

  std::vector<J::Value> items;
  CHECK(J::parseArray(" [1, \"a\\\"b\", true, null, [2], {\"x\": 1}] ", items, err) && items.size() == 6);
  CHECK(items[0].kind == J::Kind::Number && items[0].text == "1");
  CHECK(items[1].kind == J::Kind::String && items[1].text == "a\"b");
  CHECK(items[2].kind == J::Kind::Bool && items[3].kind == J::Kind::Null);
  CHECK(items[4].kind == J::Kind::Array && items[4].text == "[2]");
  CHECK(items[5].kind == J::Kind::Object && items[5].text == "{\"x\": 1}");
  CHECK(J::parseArray("[]", items, err) && items.empty());
  CHECK(!J::parseArray("[1,]", items, err));
  CHECK(!J::parseArray("[1] x", items, err));
  CHECK(!J::parseArray("{\"a\":1}", items, err));

  // Extractor: one pass, the same results as get() for every path
  {
    const char *docs[] = {
      R"({"a": {"b": 1, "c": [10, {"d": "x\"y"}, 30]}, "s": "t", "arr": [{"y": 1}, {"y": 2}, {"y": 3}], "n": null})",
      R"({"dup": {"x": 1}, "dup": {"x": 2, "y": 3}, "a": [1, 2]})",
      R"({"a": {"b": 1}, "bad": [1, , 2], "z": 5})",
      R"({"a": 1, "b": "unterminated)",
      R"([0, [1, 2], {"a": true}])",
      R"(  42  )", R"(42 x)", R"("just text")", R"(hello)", R"()", R"({})", R"([])",
      R"({"a": {"b": {"c": {"d": [[[{"e": false}]]]}}}, "after": 1})",
      R"({"a": 5, "x": [1, 2, 3], "s": "é😀"} trailing)",
    };
    const char *paths[] = {"", "a", "a.b", "a.c", "a.c[1].d", "a.c[5]", "a.b.x", "s", "arr[2].y", "arr[0]", "n",
                           "dup", "dup.x", "dup.y", "bad", "z", "[0]", "[1][1]", "[2].a", "[3]", "x[2]",
                           "a.b.c.d[0][0][0].e", "after", "missing", "a[1]"};
    int mismatches = 0, compared = 0;
    for (const char *d : docs)
    {
      J::Extractor all;
      std::vector<J::Path> ps;
      std::vector<int> ids;
      for (const char *pt : paths) { J::Path pp; J::parsePath(pt, pp, err); ps.push_back(pp); ids.push_back(all.add(pp)); }
      std::vector<J::Extractor::Result> res;
      std::string xerr;
      all.run(d, res, xerr);
      for (size_t i = 0; i < ps.size(); i++)
      {
        J::Value gv;
        std::string gerr;
        J::Found gf = J::get(d, ps[i], gv, gerr);
        // all paths in one extractor, and each path alone
        J::Extractor one;
        std::vector<J::Extractor::Result> r1;
        std::string e1;
        int id1 = one.add(ps[i]);
        one.run(d, r1, e1);
        for (const J::Extractor::Result *r : {&res[(size_t) ids[i]], &r1[(size_t) id1]})
        {
          compared++;
          bool same = r->found == gf && (gf != J::Found::Yes || (r->value.kind == gv.kind && r->value.text == gv.text));
          if ( !same )
          {
            mismatches++;
            std::cerr << "extractor != get: doc " << d << "  path '" << paths[i] << "'  get=" << (int) gf << " '" << gv.text
                      << "'  extractor=" << (int) r->found << " '" << r->value.text << "'\n";
          }
        }
      }
    }
    CHECK(mismatches == 0);
    CHECK(compared == 2 * 14 * 25);

    // every document cut off at every position: errors at every point
    int cutMismatches = 0, cutCompared = 0;
    for (const char *d : docs)
    {
      std::string full(d);
      J::Extractor all;
      std::vector<J::Path> ps;
      std::vector<int> ids;
      for (const char *pt : paths) { J::Path pp; J::parsePath(pt, pp, err); ps.push_back(pp); ids.push_back(all.add(pp)); }
      for (size_t cut = 0; cut <= full.size(); cut++)
      {
        std::string doc = full.substr(0, cut);
        std::vector<J::Extractor::Result> res;
        std::string xerr;
        all.run(doc, res, xerr);
        for (size_t i = 0; i < ps.size(); i++)
        {
          J::Value gv;
          std::string gerr;
          J::Found gf = J::get(doc, ps[i], gv, gerr);
          const auto &r = res[(size_t) ids[i]];
          cutCompared++;
          if ( r.found != gf || (gf == J::Found::Yes && (r.value.kind != gv.kind || r.value.text != gv.text)) )
          {
            if ( cutMismatches++ < 5 )
              std::cerr << "cut " << cut << " of " << full << "  path '" << paths[i] << "'  get=" << (int) gf
                        << "  extractor=" << (int) r.found << "\n";
          }
        }
      }
    }
    CHECK(cutMismatches == 0);
    std::cout << "extractor vs get: " << compared + cutCompared << " comparisons\n";

    J::Extractor x;
    J::Path pa, pb;
    J::parsePath("a.b", pa, err);
    J::parsePath("a.b", pb, err);
    CHECK(x.add(pa) == 0 && x.add(pb) == 0 && x.size() == 1);  // same path, same id
  }

  long long ms = 0;
  CHECK(J::parseTime({J::Kind::Number, "1700000000"}, ms) && ms == 1700000000000LL);
  CHECK(J::parseTime({J::Kind::Number, "1700000000123"}, ms) && ms == 1700000000123LL);
  CHECK(J::parseTime({J::Kind::Number, "1700000000.5"}, ms) && ms == 1700000000500LL);
  CHECK(J::parseTime({J::Kind::String, "2023-11-14T22:13:20.123Z"}, ms) && ms == 1700000000123LL);
  CHECK(J::parseTime({J::Kind::String, "2023-11-14T23:13:20+01:00"}, ms) && ms == 1700000000000LL);
  CHECK(J::parseTime({J::Kind::String, "2023-11-14 22:13:20"}, ms) && ms == 1700000000000LL);
  CHECK(!J::parseTime({J::Kind::String, "yesterday"}, ms));
  CHECK(!J::parseTime({J::Kind::Bool, "true"}, ms));
  CHECK(J::isoTime(1700000000123LL) == "2023-11-14T22:13:20.123Z");
  CHECK(J::isoTime(0) == "1970-01-01T00:00:00.000Z");

  // ---------------------------------------------------------- redundancy
  {
    namespace R = RsMqttRedu;
    CHECK(R::forHost("_RsMqtt_plant", false) == "_RsMqtt_plant");
    CHECK(R::forHost("_RsMqtt_plant", true) == "_RsMqtt_plant_2");
    CHECK(R::forHost("_RsMqttDriver_2", true) == "_RsMqttDriver_2_2");
    CHECK(R::endsWithSuffix("line_2") && !R::endsWithSuffix("line_12x") && !R::endsWithSuffix("_2") && !R::endsWithSuffix("line2"));
    std::string opt;
    CHECK(R::splitReduOption("redu.serverURI", opt) && opt == "serverURI");
    CHECK(R::splitReduOption("REDU.clientId", opt) && opt == "clientId");
    CHECK(!R::splitReduOption("redu.", opt) && !R::splitReduOption("serverURI", opt) && !R::splitReduOption("reduce", opt));
    CHECK(R::clientIdForHost("WCCOArsmqtt_3_plant", false, false) == "WCCOArsmqtt_3_plant");
    CHECK(R::clientIdForHost("WCCOArsmqtt_3_plant", true, false) == "WCCOArsmqtt_3_plant_2");
    CHECK(R::clientIdForHost("scada-b", true, true) == "scada-b");   // redu.clientId as given
    CHECK(R::clientIdForHost("", true, false) == "");                // broker assigns one
  }

  if ( failures )
  {
    std::cerr << failures << " check(s) failed\n";
    return 1;
  }
  std::cout << "address_test: all checks passed\n";
  return 0;
}
