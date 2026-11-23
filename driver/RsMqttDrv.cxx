#include "RsMqttDrv.hxx"
#include "RsMqttHWMapper.hxx"
#include "RsMqttHWService.hxx"

#include <HWObject.hxx>

void RsMqttDrv::install_HWMapper()
{
  hwMapper = new RsMqttHWMapper;
}

void RsMqttDrv::install_HWService()
{
  hwService = new RsMqttHWService;
}

void RsMqttDrv::install_AlertService()
{
  DrvManager::install_AlertService();
}

HWObject *RsMqttDrv::getHWObject() const
{
  return new HWObject();
}
