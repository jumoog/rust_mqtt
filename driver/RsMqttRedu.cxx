#include "RsMqttRedu.hxx"

#include <Resources.hxx>

namespace RsMqttRedu
{
  static int simulated = 0;  // 0: ask WinCC OA, 1 / 2: act as that host (tests)

  void simulateHost(int host)
  {
    simulated = (host == 1 || host == 2) ? host : 0;
  }

  int simulatedHost()
  {
    return simulated;
  }

  bool isRedundant()
  {
    return simulated ? true : Resources::isRedundant();
  }

  bool isHost2()
  {
    if ( simulated )
      return simulated == 2;
    return Resources::isRedundant() && Resources::getMyReduHostNum() == 2;
  }
}
