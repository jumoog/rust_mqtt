// Unit test of the driver's RsMqttAddress (parser and topic matcher).
#include "RsMqttAddress.hxx"

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

  if ( failures )
  {
    std::cerr << failures << " check(s) failed\n";
    return 1;
  }
  std::cout << "address_test: all checks passed\n";
  return 0;
}
