// RsMqttResources.hxx - command line and config file of the rsmqtt driver.
//
// [rsmqtt]
// maxPayload = 65536            # max. size of a published value (bytes)
// traceLevel = 0                # rsmqtt/Paho trace into the log, 0..5
// driverDp = "_RsMqttDriver_2"  # DP listing the broker DPs (default _RsMqttDriver_<num>)
//
// Brokers can be datapoints (_RsMqttBroker, see RsMqttBrokerDp.hxx) and/or
// be configured here:
//
// connection = "plant"          # starts the settings of connection "plant"
// serverURI  = "tcp://broker:1883"
// MQTTVersion = 5
// username = "..."  password = "..."
// ssl.trustStore = "C:/certs/ca.pem"       (any rsmqtt option, see rsmqtt.h)
// subscribeQoS = 1              # driver: default QoS of input addresses
// publishQoS = 1                # driver: default QoS of output addresses
// publishRetain = 0             # driver: default retain flag of outputs
// reconnectInterval = 10        # driver: retry of failed connects (s)
//
// connection = "cloud"
// serverURI = "mqtts://example.com:8883"
// ...
#ifndef RSMQTT_RESOURCES_HXX
#define RSMQTT_RESOURCES_HXX

#include <DrvResources.hxx>

class RsMqttConnection;

class RsMqttResources : public DrvResources
{
  public:
    static void init(int &argc, char *argv[]);
    static PVSSboolean readSection();

    static unsigned getMaxPayload() { return maxPayload; }
    static int      getTraceLevel() { return traceLevel; }
    /// _RsMqttDriver DP listing the brokers (default _RsMqttDriver_<num>)
    static const CharString &getDriverDp() { return driverDp; }

  private:
    static unsigned maxPayload;
    static int      traceLevel;
    static CharString driverDp;
    static RsMqttConnection *current;
};

#endif
