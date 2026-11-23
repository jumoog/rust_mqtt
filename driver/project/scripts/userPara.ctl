//////////////////////////////////////////////////////////////////////////////
// userPara.ctl - PARA address panel functions of user drivers.
//
// This file contains the "rsmqtt" cases for the WCCOArsmqtt MQTT driver
// (panel para/address_rsmqtt.pnl). A project has only ONE userPara.ctl: if
// yours already handles other drivers, copy the four  case "rsmqtt":  blocks
// (and the #uses line) into the same functions of your file.
//
// dpc[] layout used by PARA:
//   dpc[1]=reference  dpc[2]=subindex  dpc[3]=mode  dpc[7]=datatype
//   dpc[8]=drv_ident  dpc[9]=driver    dpc[12]=active
//////////////////////////////////////////////////////////////////////////////

#uses "rsmqtt"

//////////////////////////////////////////////////////////////////////////////
// fct = 1: upDpGetAddress
// fct = 2: upDpSetAddress
// fct = 3: upWritePanelAllAddressAttributes
// fct = 4: upReadPanelAllAddressAttributes
//////////////////////////////////////////////////////////////////////////////
anytype main(string dpe, int Id, anytype dpc, int fct)
{
  bool all_right;

  switch (fct)
  {
    case 1: upDpGetAddress(dpe, Id, dpc);
      break;
    case 2: upDpSetAddress(dpe, Id, dpc, all_right); dpc[99] = all_right;
      break;
    case 3: upWritePanelAllAddressAttributes(dpe, Id, dpc);
      break;
    case 4: upReadPanelAllAddressAttributes(dpe, Id, dpc, all_right); dpc[99] = all_right;
      break;
  }
  return dpc;
}

//////////////////////////////////////////////////////////////////////////////
// read the address config of the DPE into dpc
//////////////////////////////////////////////////////////////////////////////
upDpGetAddress(string dpe, int Id, anytype &dpc)
{
  string config = paGetDpConfig(globalOpenConfig[Id]);

  switch (globalAddressOld[Id])
  {
    case "rsmqtt":
    {
      bool     active;
      string   reference, drv_ident;
      char     mode;
      int      datatype, driver;
      unsigned subindex;

      dpGet(dpe + ":" + config + ".._active",    active,
            dpe + ":" + config + ".._reference", reference,
            dpe + ":" + config + ".._subindex",  subindex,
            dpe + ":" + config + ".._mode",      mode,
            dpe + ":" + config + ".._datatype",  datatype,
            dpe + ":" + config + ".._drv_ident", drv_ident,
            dpe + ":_distrib.._driver",          driver);

      dpc[1]  = reference;
      dpc[2]  = subindex;
      dpc[3]  = mode;
      dpc[7]  = datatype;
      dpc[8]  = drv_ident;
      dpc[9]  = (driver < 1) ? 1 : driver;
      dpc[12] = active;
      break;
    }
    default: break;
  }
}

//////////////////////////////////////////////////////////////////////////////
// write dpc into the address config of the DPE
//////////////////////////////////////////////////////////////////////////////
upDpSetAddress(string dpe, int Id, dyn_anytype dpc, bool &all_right)
{
  string config = paGetDpConfig(globalOpenConfig[Id]);

  switch (globalAddressOld[Id])
  {
    case "rsmqtt":
      paErrorHandlingDpSet(
        dpSetWait(dpe + ":_distrib.._driver", dpc[9]), all_right);
      paErrorHandlingDpSet(
        dpSetWait(dpe + ":_address.._type", DPCONFIG_PERIPH_ADDR_MAIN,
                  dpe + ":" + config + ".._reference", dpc[1],
                  dpe + ":" + config + ".._subindex",  0,
                  dpe + ":" + config + ".._mode",      dpc[3],
                  dpe + ":" + config + ".._datatype",  dpc[7],
                  dpe + ":" + config + ".._drv_ident", dpc[8]), all_right);
      paErrorHandlingDpSet(
        dpSetWait(dpe + ":_address.._active", dpc[12]), all_right);
      break;
    default: break;
  }
}

//////////////////////////////////////////////////////////////////////////////
// show dpc in the panel
//////////////////////////////////////////////////////////////////////////////
upWritePanelAllAddressAttributes(string dpe, int Id, anytype dpc)
{
  switch (globalAddressOld[Id])
  {
    case "rsmqtt":
    {
      string broker, topic, extra;
      int    qos, retain, mode = dpc[3], dir, datatype = dpc[7];
      bool   lowlevel;

      rsmqttParseReference(dpc[1], broker, topic, qos, retain, extra);

      // mode: +64 = low level comparison
      if ( mode >= 64 ) { mode -= 64; lowlevel = TRUE; }
      if ( mode >= 32 ) mode -= 32;
      // _mode: 1 output, 5 output single, 6..8 input/output, else input
      if ( mode == 1 || mode == 5 )
        dir = 1;  // output
      else if ( mode >= 6 && mode <= 8 )
        dir = 3;  // in/out
      else
        dir = 2;  // input (also for new addresses)

      if ( qos < -1 || qos > 2 ) qos = -1;

      // datatype 1001..1008 -> positions 1..8; undefined -> text
      if ( datatype < 1001 || datatype > 1008 ) datatype = 1001;

      dyn_string brokers = rsmqttGetAllBrokers();
      if ( broker != "" && dynContains(brokers, broker) < 1 )
        dynAppend(brokers, broker);  // e.g. a broker from the config file

      setMultiValue("cmbBroker",  "items",       brokers,
                    "cmbBroker",  "text",        broker,
                    "txtTopic",   "text",        topic,
                    "txtOptions", "text",        extra,
                    "cmbQos",     "selectedPos", qos + 2,      // -1 -> 1 (default)
                    "cmbRetain",  "selectedPos", retain + 2,   // -1 -> 1 (default)
                    "trans_art",  "selectedPos", datatype - 1000,
                    "einaus",     "selectedPos", dir,
                    "lowlevel",   "state", 0,    lowlevel,
                    "lowlevel",   "visible",     dir != 1,
                    "cboAddressActive", "state", 0, dpc[12],
                    "Treiber",    "text",        dpc[9],
                    "lblBrokerState", "text",    rsmqttBrokerStatusLine(broker),
                    "lblBrokerState", "foreCol", "_3DText");
      break;
    }
    default:
      break;
  }
}

//////////////////////////////////////////////////////////////////////////////
// read the panel into dpc
//////////////////////////////////////////////////////////////////////////////
upReadPanelAllAddressAttributes(string dpe, int Id, dyn_anytype &dpc, bool &readOK)
{
  readOK = TRUE;

  switch (globalAddressOld[Id])
  {
    case "rsmqtt":
    {
      string broker, topic, extra;
      int    qosPos, retainPos, typePos, dir, driver, mode;
      bool   active, lowlevel;

      getMultiValue("cmbBroker",  "text",        broker,
                    "txtTopic",   "text",        topic,
                    "txtOptions", "text",        extra,
                    "cmbQos",     "selectedPos", qosPos,
                    "cmbRetain",  "selectedPos", retainPos,
                    "trans_art",  "selectedPos", typePos,
                    "einaus",     "selectedPos", dir,
                    "lowlevel",   "state", 0,    lowlevel,
                    "cboAddressActive", "state", 0, active,
                    "Treiber",    "text",        driver);

      broker = strltrim(strrtrim(broker));
      extra  = strltrim(strrtrim(extra), ";");

      // PARA also reads the panel right after a new address config was
      // opened; nothing entered yet is not an error at that point.
      bool untouched = (broker == "" && topic == "");

      string err;
      if ( untouched )
        err = "";
      else if ( !rsmqttIsValidBrokerName(broker) )
        err = "Please select a broker (letters, digits, '_' and '-').";
      else if ( topic == "" )
        err = "Please enter a topic.";
      else if ( dir == 1 && (strpos(topic, "+") >= 0 || strpos(topic, "#") >= 0 || topic == "$state") )
        err = "An output cannot publish to a topic filter or to $state.";
      else if ( strpos(topic, ";") >= 0 )
        err = "The topic must not contain ';'.";

      if ( err != "" )
      {
        // PARA shows its own "invalid value" dialog when readOK is FALSE;
        // the reason goes into the panel's status line.
        setMultiValue("lblBrokerState", "text", err,
                      "lblBrokerState", "foreCol", "{200,0,0}");
        readOK = FALSE;
        return;
      }

      switch ( dir )
      {
        case 1:  mode = DPATTR_ADDR_MODE_OUTPUT_SINGLE; break;
        case 3:  mode = DPATTR_ADDR_MODE_IO_SPONT;      break;
        default: mode = DPATTR_ADDR_MODE_INPUT_SPONT;   break;
      }
      if ( lowlevel && dir != 1 ) mode += 64;

      dpc[1]  = untouched ? "" : rsmqttMakeReference(broker, topic, qosPos - 2, retainPos - 2, extra);
      dpc[2]  = 0;
      dpc[3]  = mode;
      dpc[7]  = (typePos < 1 ? 1 : typePos) + 1000;
      dpc[8]  = globalAddressDrivers[dynContains(globalAddressTypes, globalAddressNew[paMyModuleId()])];
      dpc[9]  = driver;
      dpc[12] = active;
      break;
    }
    default: break;
  }
}
