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
//                             no path: the whole payload (see RsMqttJson.hxx)
//                ts=path      with json: timestamp at path; input sets the
//                             value's source time, output adds the value time
//
// Examples:  plant:line1/temp            plant:line1/+/state;qos=2
//            plant:line1/setpoint;qos=1;retain   plant:$state
//            plant:sensor/1;json=data.temp;ts=data.time   plant:cmd;json=values[0]
//
// This file has no WinCC OA dependencies.
#ifndef RSMQTT_ADDRESS_HXX
#define RSMQTT_ADDRESS_HXX

#include "RsMqttJson.hxx"

#include <string>
#include <string_view>

struct RsMqttAddress
{
  std::string connection;
  std::string topic;
  int  qos = -1;         // -1: connection default
  int  retain = -1;      // -1: connection default
  int  subFlags = 0;     // RSMQTT_SUB_* flags
  bool isState = false;  // "$state" pseudo topic
  bool json = false;     // json[=path]
  RsMqttJson::Path jsonPath;
  bool hasTs = false;    // ts=path (needs json)
  RsMqttJson::Path tsPath;
  bool dyn = false;      // set by the HW mapper for dyn DPEs, not part of the syntax

  /// Parses an address; on failure returns false and sets err.
  static bool parse(std::string_view text, RsMqttAddress &out, std::string &err);

  /// Filter needs matching (wildcards or shared subscription) instead of an
  /// exact topic comparison.
  bool needsMatching() const { return isWildcardFilter(topic); }

  /// "<connection>:<topic>", the address without options.
  std::string key() const { return connection + ":" + topic; }

  static bool hasWildcards(std::string_view filter);
  static bool isWildcardFilter(std::string_view filter);

  /// MQTT topic matching (including $share/<group>/ filters and the rule
  /// that wildcards at the first level do not match $-topics).
  static bool topicMatches(std::string_view filter, std::string_view topic);
};

#endif
