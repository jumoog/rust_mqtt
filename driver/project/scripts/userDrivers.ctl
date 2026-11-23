//////////////////////////////////////////////////////////////////////////////
// userDrivers.ctl - registers user drivers in the PARA address panel.
//
// This file contains the entry for the rsmqtt MQTT driver (WCCOArsmqtt).
// A project has only ONE userDrivers.ctl: if yours already registers other
// drivers, append the three "RSMQTT" values below to the same positions of
// its dyn_strings instead of replacing the file.
//
//   names   : text in the driver type combobox of para/address.pnl
//   types   : panel para/address_<type>.pnl  ->  address_rsmqtt.pnl
//   drivers : value written to _address.._drv_ident
//////////////////////////////////////////////////////////////////////////////

dyn_dyn_string main()
{
  dyn_dyn_string dds;

  // names
  dds[1] = makeDynString("MQTT (rsmqtt)");

  // types
  dds[2] = makeDynString("rsmqtt");

  // drivers
  dds[3] = makeDynString("RSMQTT");

  return dds;
}
