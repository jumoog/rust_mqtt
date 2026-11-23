#ifndef RSMQTT_DRV_HXX
#define RSMQTT_DRV_HXX

#include <DrvManager.hxx>

class RsMqttDrv : public DrvManager
{
  public:
    void install_HWMapper() override;
    void install_HWService() override;
    void install_AlertService() override;
    HWObject *getHWObject() const override;
};

#endif
