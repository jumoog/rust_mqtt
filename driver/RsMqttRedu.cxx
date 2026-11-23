#include "RsMqttRedu.hxx"

#include <Resources.hxx>

namespace RsMqttRedu
{
  bool isRedundant()
  {
    return Resources::isRedundant();
  }

  bool isHost2()
  {
    return Resources::isRedundant() && Resources::getMyReduHostNum() == 2;
  }
}
