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
// The functions in the first part have no WinCC OA dependencies.
#ifndef RSMQTT_REDU_HXX
#define RSMQTT_REDU_HXX

#include <cctype>
#include <string>

namespace RsMqttRedu
{
  constexpr const char *SUFFIX = "_2";

  /// The name a host uses: "<base>" on host 1, "<base>_2" on host 2.
  inline std::string forHost(const std::string &base, bool host2)
  {
    return host2 ? base + SUFFIX : base;
  }

  /// TRUE if a name ends with "_2", which is the host-2 twin of another name.
  inline bool endsWithSuffix(const std::string &name)
  {
    return name.size() > 2 && name.compare(name.size() - 2, 2, SUFFIX) == 0;
  }

  /// "redu.<option>" (any case): TRUE and option = "<option>".
  inline bool splitReduOption(const std::string &name, std::string &option)
  {
    if ( name.size() <= 5 ) return false;
    for (size_t i = 0; i < 5; i++)
      if ( std::tolower((unsigned char) name[i]) != "redu."[i] ) return false;
    option = name.substr(5);
    return true;
  }

  /// The client id a host connects with. An empty id (assigned by the
  /// broker) stays empty; an explicit redu.clientId is used as it is.
  inline std::string clientIdForHost(const std::string &clientId, bool host2, bool reduClientIdGiven)
  {
    if ( !host2 || reduClientIdGiven || clientId.empty() ) return clientId;
    return clientId + SUFFIX;
  }

  //----------------------------------------------------------------- WinCC OA

  /// The project is redundant (two event managers).
  bool isRedundant();
  /// This driver runs on the second host of a redundant system.
  bool isHost2();
}

#endif
