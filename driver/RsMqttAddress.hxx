// RsMqttAddress.hxx - peripheral address syntax of the rsmqtt driver.
//
//   <connection>:<topic>[;option[;option...]]
//
//   connection   name of a connection from the [rsmqtt] config section
//   topic        MQTT topic; for input addresses also a topic filter with
//                + / # wildcards or a shared subscription ($share/group/...)
//                "$state" is the connection state (1 = connected, 0 = not)
//   options      qos=0|1|2    subscribe QoS (input) or publish QoS (output)
//                retain[=0|1] publish retained (output)
//                nolocal      v5: don't receive own publications
//                rap          v5: retain as published
//                rh=0|1|2     v5: retain handling
//                json[=path]  payload is JSON: the value is at path (input),
//                             or is published nested along path (output);
//                             no path: the whole payload
//                ts=path      with json: timestamp at path; input sets the
//                             value's source time, output adds the value time
//
// Sparkplug B (host application):
//   <connection>:$spb/<group>/<edge node>[/<device>];metric=<name>
//                input: the metric's value; output: NCMD / DCMD
//   <connection>:$spb/<group>/<edge node>[/<device>];online
//                input: 1 while the edge node / device is born, else 0
//
// Examples:  plant:line1/temp            plant:line1/+/state;qos=2
//            plant:line1/setpoint;qos=1;retain   plant:$state
//            plant:sensor/1;json=data.temp;ts=data.time   plant:cmd;json=values[0]
//            plant:$spb/Factory/Line1/PLC7;metric=Motor/speed
//
// The parser is implemented in Rust (crates/rsmqtt-wincc/src/address.rs).
#ifndef RSMQTT_ADDRESS_HXX
#define RSMQTT_ADDRESS_HXX

#include <rsmqtt_wincc.hpp>

#include <string>
#include <string_view>

struct RsMqttAddress : rsmqtt::wincc::Address
{
  bool dyn = false;      // set by the HW mapper for dyn DPEs, not part of the syntax
  bool input = true;     // set by the HW mapper: the DPE receives values

  /// Parses an address; on failure returns false and sets err.
  static bool parse(std::string_view text, RsMqttAddress &out, std::string &err)
  {
    out = RsMqttAddress();
    return rsmqtt::wincc::Address::parse(text, out, err);
  }

  /// Filter needs matching (wildcards or shared subscription) instead of an
  /// exact topic comparison.
  bool needsMatching() const { return isWildcardFilter(topic); }

  /// "<connection>:<topic>", the address without options.
  std::string key() const { return connection + ":" + topic; }

  static bool isWildcardFilter(std::string_view filter) { return rsmqtt::wincc::isWildcardFilter(filter); }

  /// MQTT topic matching (including $share/<group>/ filters and the rule
  /// that wildcards at the first level do not match $-topics).
  static bool topicMatches(std::string_view filter, std::string_view topic)
  {
    return rsmqtt::wincc::topicMatches(filter, topic);
  }
};

#endif
