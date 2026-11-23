#ifndef RSMQTT_HWSERVICE_HXX
#define RSMQTT_HWSERVICE_HXX

#include <HWService.hxx>

#include <chrono>
#include <string>
#include <string_view>

class RsMqttConnection;

class RsMqttHWService : public HWService
{
  public:
    PVSSboolean initialize(int argc, char *argv[]) override;
    PVSSboolean start() override;
    void stop() override;
    void workProc() override;
    PVSSboolean writeData(HWObject *objPtr) override;

  private:
    void createClient(RsMqttConnection &conn);
    void teardown(RsMqttConnection &conn);
    void reconfigure(RsMqttConnection &conn);
    void connect(RsMqttConnection &conn);
    void handleEvents(RsMqttConnection &conn);
    void deliver(const std::string &address, std::string_view payload);
    void deliverMessage(RsMqttConnection &conn, const std::string &topic, std::string_view payload);
    void pushState(RsMqttConnection &conn);
    void publishBrokerState(RsMqttConnection &conn, bool force);
    void drainTrace();

    bool started = false;
    std::chrono::steady_clock::time_point startTime{};
};

#endif
