// RsMqttSparkplug.hxx - Sparkplug B (v3.0) Host Application, implemented in
// Rust (crates/rsmqtt-wincc/src/sparkplug.rs, protobuf types from srad-types);
// see rsmqtt_wincc.hpp.
//
// The driver subscribes spBv1.0/<group>/+/<edge node>/# per edge node and hands
// every message to Host::onMessage(), which returns the updates for the
// addresses (values, online state, stale), the rebirth requests to publish and
// warnings. Host::command() builds NCMD / DCMD with the data type from the birth.
#ifndef RSMQTT_SPARKPLUG_HXX
#define RSMQTT_SPARKPLUG_HXX

#include <rsmqtt_wincc.hpp>

namespace RsMqttSparkplug = rsmqtt::wincc::sparkplug;

#endif
