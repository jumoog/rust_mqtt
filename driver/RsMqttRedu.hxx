// RsMqttRedu.hxx - WinCC OA redundancy (two hosts, one active).
//
// As the WinCC OA drivers do it:
//   - Every internal DP exists twice: host 1 uses "<dp>", host 2 "<dp>_2"
//     (_RsMqttDriver_3 / _RsMqttDriver_3_2, _RsMqtt_plant / _RsMqtt_plant_2).
//     The broker names in Brokers stay the same on both hosts.
//   - Config, Command and the broker list are written to "<dp>" only; the
//     event manager copies them to "<dp>_2" (copyDpType in config.redu).
//     State is forwarded to the other system (fwdDpType).
//   - Both drivers connect and read; ComDrv drops outputs on the passive host.
//   - Both drivers must not share a client id (the broker would disconnect
//     one whenever the other connects): host 2 appends "_2" to it.
//   - "redu.<option>=<value>" (broker Config.Options or the config file)
//     applies to host 2 only, e.g. redu.serverURI or redu.clientId; with
//     redu.clientId the "_2" is not appended.
//
// The name rules are implemented in Rust (crates/rsmqtt-wincc/src/redu.rs);
// which host this is comes from WinCC OA (RsMqttRedu.cxx).
#ifndef RSMQTT_REDU_HXX
#define RSMQTT_REDU_HXX

#include <rsmqtt_wincc.hpp>

#include <string>

namespace RsMqttRedu
{
  using rsmqtt::wincc::redu::forHost;
  using rsmqtt::wincc::redu::endsWithSuffix;
  using rsmqtt::wincc::redu::splitReduOption;
  using rsmqtt::wincc::redu::clientIdForHost;

  /// The project is redundant (two event managers).
  bool isRedundant();
  /// This driver runs on the second host of a redundant system.
  bool isHost2();

  /// For tests without a redundant system (config "simulateReduHost = 1|2"):
  /// the driver acts as that host of a redundant pair. 0 turns it off.
  void simulateHost(int host);
  int simulatedHost();
}

#endif
