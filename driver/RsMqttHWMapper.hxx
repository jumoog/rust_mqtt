#ifndef RSMQTT_HWMAPPER_HXX
#define RSMQTT_HWMAPPER_HXX

#include <TreeHWMapper.hxx>

class RsMqttHWMapper : public TreeHWMapper
{
  public:
    PVSSboolean addDpPa(DpIdentifier &dpId, PeriphAddr *confPtr) override;
    PVSSboolean clrDpPa(DpIdentifier &dpId, PeriphAddr *confPtr) override;
};

#endif
