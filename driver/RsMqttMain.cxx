// WCCOArsmqtt - WinCC OA MQTT driver based on the rsmqtt (Rust) library.

#include "RsMqttDrv.hxx"
#include "RsMqttResources.hxx"

#include <Utf8Argv.hxx>

#include <signal.h>

int main(int argc, char **argv)
{
  Utf8Argv utf8Argv(argc, argv);

  RsMqttResources resources;
  resources.init(argc, argv);

  // Driver type on _DriverCommon.DT
  RsMqttResources::setDriverType("RSMQTT");

  if ( resources.getHelpFlag() )
  {
    resources.printHelp();
    return 0;
  }
  if ( resources.getHelpDbgFlag() )
  {
    resources.printHelpDbg();
    return 0;
  }
  if ( resources.getHelpReportFlag() )
  {
    resources.printHelpReport();
    return 0;
  }

  signal(SIGINT, Manager::sigHdl);
  signal(SIGTERM, Manager::sigHdl);

  // a pointer is needed, since the Manager dtor does a delete
  RsMqttDrv *driver = new RsMqttDrv;
  driver->mainProcedure(argc, argv);

  return 0;
}
