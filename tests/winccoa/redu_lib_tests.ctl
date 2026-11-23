// Checks of the redundancy functions in scripts/libs/rsmqtt.ctl.
// Appended to the library source and run with evalScript(); `simulated`
// TRUE means the caller replaced isRedundant() with TRUE in the library.
//
//   string lib, t; fileToString(".../rsmqtt.ctl", lib); fileToString(".../redu_lib_tests.ctl", t);
//   evalScript(res, lib + t, makeDynString(), FALSE);    // single system
//   strreplace(lib, "isRedundant()", "TRUE");
//   evalScript(res, lib + t, makeDynString(), TRUE);     // redundant paths

dyn_string gRes;

void check(string name, bool ok, string detail = "")
{
  dynAppend(gRes, (ok ? "PASS  " : "FAIL  ") + name + (detail != "" ? "  | " + detail : ""));
}

dyn_string main(bool simulated)
{
  string pre = simulated ? "[redundant] " : "[single] ";
  // twins that exist before: everything else this test creates is removed at the end
  dyn_string twinsBefore = dpNames("*_2", "_RsMqttBroker");
  dynAppend(twinsBefore, dpNames("*_2", "_RsMqttDriver"));

  check(pre + "new broker names must not end with _2",
        !rsmqttIsNewBrokerName("line_2") && rsmqttIsNewBrokerName("line") && rsmqttIsValidBrokerName("line_2"));
  check(pre + "rsmqttCreateBroker refuses a new 'x_2'",
        rsmqttCreateBroker("o_lib_2", 3, "tcp://localhost:18840") == -1 && !dpExists("_RsMqtt_o_lib_2"));

  // create a broker: in a redundant project the twin comes first, with all values
  rsmqttCreateBroker("o_lib", 3, "tcp://localhost:18840", makeMapping("KeepAlive", 17, "Options", makeDynString("maxInflight=10", "connectTimeout=5")));
  delay(1);
  bool twin = dpExists("_RsMqtt_o_lib_2");
  if ( simulated )
  {
    string uri; int ka; dyn_string opts; bool act;
    if ( twin )
      dpGet("_RsMqtt_o_lib_2.Config.ServerURI", uri, "_RsMqtt_o_lib_2.Config.KeepAlive", ka,
            "_RsMqtt_o_lib_2.Config.Options", opts, "_RsMqtt_o_lib_2.Config.Active", act);
    // without config.redu (single system) only what was set before the copy is in the twin
    check(pre + "rsmqttCreateBroker creates the twin _RsMqtt_o_lib_2", twin, "ServerURI '" + uri + "'");
    check(pre + "driver twin _RsMqttDriver_3_2 exists", dpExists("_RsMqttDriver_3_2"));
  }
  else
    check(pre + "no twin in a single system", !twin);

  // lists never show twins
  dyn_string brokers = rsmqttGetAllBrokers();
  bool fakeTwin = FALSE;
  if ( !dpExists("_RsMqtt_o_lib_2") ) { dpCreate("_RsMqtt_o_lib_2", "_RsMqttBroker"); fakeTwin = TRUE; }
  bool fakeDrv = FALSE;
  if ( !dpExists("_RsMqttDriver_3_2") ) { dpCreate("_RsMqttDriver_3_2", "_RsMqttDriver"); fakeDrv = TRUE; }
  brokers = rsmqttGetAllBrokers();
  if ( simulated )
    check(pre + "rsmqttGetAllBrokers lists o_lib, not its twin", dynContains(brokers, "o_lib") > 0 && dynContains(brokers, "o_lib_2") == 0,
          strjoin(brokers, ","));
  else
    check(pre + "rsmqttGetAllBrokers keeps an ordinary broker o_lib_2", dynContains(brokers, "o_lib") > 0 && dynContains(brokers, "o_lib_2") > 0,
          strjoin(brokers, ","));
  dyn_string drivers = rsmqttDriverDps();
  check(pre + "rsmqttDriverDps skips _RsMqttDriver_3_2", dynContains(drivers, "_RsMqttDriver_3") > 0 && dynlen(dynPatternMatch("*_3_2", drivers)) == 0,
        strjoin(drivers, ","));
  check(pre + "rsmqttGetBrokerDriver", rsmqttGetBrokerDriver("o_lib") == 3, (string) rsmqttGetBrokerDriver("o_lib"));

  // status line: both hosts in a redundant project
  dpSetWait("_RsMqtt_o_lib_2.State.ConnState", 1);
  string line = rsmqttBrokerStatusLine("o_lib");
  check(pre + "status line " + (simulated ? "adds host 2" : "has no host part"),
        simulated ? strpos(line, "|  " + rsmqttT("Host ", "host ") + "2: " + rsmqttConnStateText(RSMQTT_CS_DISCONNECTED)) > 0
                  : strpos(strtolower(line), "|  host") < 0, line);   // texts in the UI language

  // upgrade creates missing twins (nothing in a single system)
  if ( simulated )
  {
    dpDelete("_RsMqtt_o_lib_2");
    delay(0, 300);
    int n = rsmqttUpgradeToRedundant();
    check(pre + "rsmqttUpgradeToRedundant creates the missing twin", n >= 1 && dpExists("_RsMqtt_o_lib_2"), n + " created");
    int ka;
    dpGet("_RsMqtt_o_lib_2.Config.KeepAlive", ka);
    check(pre + "  ... with the values of the original", ka == 17, "KeepAlive " + ka);
  }
  else
    check(pre + "rsmqttUpgradeToRedundant does nothing", rsmqttUpgradeToRedundant() == 0);

  // delete removes the twin too (redundant)
  rsmqttDeleteBroker("o_lib");
  delay(0, 500);
  if ( simulated )
    check(pre + "rsmqttDeleteBroker deletes the twin too", !dpExists("_RsMqtt_o_lib") && !dpExists("_RsMqtt_o_lib_2"));
  else
    check(pre + "rsmqttDeleteBroker", !dpExists("_RsMqtt_o_lib"));

  // leave nothing behind (rsmqttUpgradeToRedundant made twins of every broker)
  dyn_string twinsAfter = dpNames("*_2", "_RsMqttBroker");
  dynAppend(twinsAfter, dpNames("*_2", "_RsMqttDriver"));
  for (int i = 1; i <= dynlen(twinsAfter); i++)
    if ( dynContains(twinsBefore, twinsAfter[i]) < 1 )
      dpDelete(dpSubStr(twinsAfter[i], DPSUB_DP));
  return gRes;
}
