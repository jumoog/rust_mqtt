"""Generates the WinCC OA panels of the rsmqtt driver.

    python driver/tools/gen_panels.py

  panels/para/address_rsmqtt.pnl   PARA address panel (driver type "rsmqtt")
  panels/para/rsmqtt_brokers.pnl   broker configuration (_RsMqttBroker DPs)

Both use the classic WinCC OA V14 panel text format. The record layouts are
taken from stock panels: address_sim.pnl (which the WinCC OA SampleDriver
documentation names as the base for own driver panels) and poll_groups.pnl.
The panels are generated rather than hand-written because every LANG entry
needs its UTF-8 byte count.

Object names used by scripts/userPara.ctl (address panel):
    cmbBroker, cmdBrokers, txtTopic, cmbQos, cmbRetain, txtOptions, trans_art,
    einaus, lowlevel, Treiber, lblBrokerState (+ cboAddressActive from
    para/refAddressActive.pnl)
Object names used by the rsmqttPanel*() functions of scripts/libs/rsmqtt.ctl
(broker panel):
    cmbBroker, spinDriver, chkActive, txtUri, txtUris, txtClientId, cmbVersion,
    txtKeepAlive, chkClean, txtUser, txtPassword, txtTrust, txtKeyStore,
    txtKey, chkVerify, txtWillTopic, txtWillPayload, cmbWillQos,
    chkWillRetain, cmbSubQos, cmbPubQos, chkPubRetain, txtOptions, cmbAddOption,
    lblState, lblError, lblInfo, lblMessage, txtSpbHost, txtSpbHost2, lblSpbState,
    tabBrokers, lblShown, lblSnapshot, lblCopyFrom
"""

import re
import sys
from pathlib import Path

PANELS = Path(__file__).resolve().parents[1] / "project" / "panels" / "para"
OPTIONS_RS = Path(__file__).resolve().parents[2] / "crates" / "rsmqtt" / "src" / "options.rs"

# The "add an option" list of the broker panel: every rsmqtt option that has
# no field of its own, as "name=example value". Checked against OPTION_NAMES
# in crates/rsmqtt/src/options.rs (see check_option_list).
PANEL_OPTIONS = [
    # connection
    "connectTimeout=30",
    "automaticReconnect=1",
    "minRetryInterval=1",
    "maxRetryInterval=60",
    "maxInflight=65535",
    "httpProxy=http://proxy:8080",
    "httpsProxy=http://proxy:8080",
    "httpHeader=Name: value",
    # MQTT 5 enhanced authentication (user name and password of the form)
    "auth.method=SCRAM-SHA-256",
    "auth.method=SCRAM-SHA-512",
    "auth.method=SCRAM-SHA-1",
    # MQTT 5 connect properties
    "sessionExpiryInterval=3600",
    "receiveMaximum=65535",
    "maximumPacketSize=268435455",
    "topicAliasMaximum=0",
    "requestResponseInformation=0",
    "requestProblemInformation=1",
    "userProperty=key=value",
    # buffering and persistence
    "sendWhileDisconnected=1",
    "maxBufferedMessages=10000",
    "deleteOldestMessages=1",
    "allowDisconnectedSendAtAnyTime=0",
    "persistence=file",
    "persistenceDir=rsmqtt",
    "restoreMessages=1",
    "persistQos0=0",
    # last will (MQTT 5)
    "will.delayInterval=0",
    "will.messageExpiryInterval=3600",
    "will.contentType=application/json",
    "will.responseTopic=topic",
    "will.payloadFormatIndicator=1",
    "will.userProperty=key=value",
    # TLS
    "ssl.enableServerCertAuth=1",
    "ssl.sslVersion=1.2",
    "ssl.enabledCipherSuites=TLS13_AES_256_GCM_SHA384",
    "ssl.alpn=mqtt",
    "ssl.CApath=C:/certs",
    "ssl.disableDefaultTrustStore=0",
    # redundancy: settings of the second host
    "redu.serverURI=tcp://broker-b:1883",
    "redu.clientId=scada-b",
    "redu.username=user",
]

# rsmqtt options with a field of their own in the broker panel, or not for the
# driver: auth.data only goes with a C authentication callback, the event
# options are the driver's, ssl.privateKeyPassword is not supported
NOT_IN_PANEL_LIST = {
    "serveruri", "serveruris", "clientid", "mqttversion", "keepaliveinterval", "cleansession", "cleanstart",
    "username", "password", "ssl.truststore", "ssl.keystore", "ssl.privatekey", "ssl.verify",
    "will.topic", "will.payload", "will.message", "will.qos", "will.retained",
    "auth.data", "completionevents", "maxqueuedevents", "runtimethreads", "ssl.privatekeypassword",
}


def normalize_option(name):
    """As rsmqtt's normalize_name for the names used here: lower case, no _ or -."""
    return name.strip().lower().replace("_", "").replace("-", "")


def check_option_list():
    """Every listed option is an rsmqtt option, and every rsmqtt option is listed or deliberately left out."""
    src = OPTIONS_RS.read_text(encoding="utf-8")
    block = re.search(r"pub const OPTION_NAMES: &\[&str\] = &\[(.*?)\];", src, re.S).group(1)
    known = set(re.findall(r'"([a-z0-9.]+)"', block))
    listed = set()
    for opt in PANEL_OPTIONS:
        name = normalize_option(opt.split("=", 1)[0])
        base = name[len("redu."):] if name.startswith("redu.") else name
        if base not in known:
            sys.exit(f"gen_panels: '{opt}' is not an rsmqtt option ({OPTIONS_RS})")
        if not name.startswith("redu."):
            listed.add(name)
    missing = known - listed - NOT_IN_PANEL_LIST
    if missing:
        sys.exit("gen_panels: rsmqtt options missing from PANEL_OPTIONS (or NOT_IN_PANEL_LIST): " +
                 ", ".join(sorted(missing)))

LANGS = (10000, 10001, 10027)  # de, en, ru (as the stock PARA panels)

FONT_BOLD = "Arial,-1,13,5,700,0,0,0,0,0,0,0,0,0,0,1"
FONT_NORMAL = "Arial,-1,13,5,400,0,0,0,0,0,0,0,0,0,0,1"
FONT_TITLE = "arial,-1,18,5,400,0,0,0,0,0,0,0,0,0,0,1"


def lang(de, en=None, ru=None):
    """Three LANG lines with UTF-8 byte counts."""
    en = de if en is None else en
    ru = en if ru is None else ru
    return [f"LANG:{code} {len(text.encode('utf-8'))} {text}" for code, text in zip(LANGS, (de, en, ru))]


def fonts(font):
    return ["3"] + [f"LANG:{code} {len(font)} {font}" for code in LANGS]


def tip(de="", en=None):
    return ["0", "3"] + lang(de, en) + [""]


def script(src):
    """A CTRL script as a quoted panel string."""
    return '"' + src.replace("\\", "\\\\").replace('"', '\\"') + '" 0'


class Panel:
    """Collects the shape records of one panel."""

    def __init__(self):
        self.shape = 100   # shape ids (stock reference placeholders use 66..77)
        self.serial = 100  # tab order (stock shapes copied below use < 100)
        self.records = []
        self.ox = 0        # added to every x (the broker form sits right of its table)

    def next(self):
        self.shape += 1
        self.serial += 1
        return self.shape, self.serial

    def add(self, rec):
        self.records += rec

    # ----------------------------------------------------------- shapes

    def label(self, name, x, y, w, de, en=None, ru=None, bold=True):
        sid, ser = self.next()
        x += self.ox
        self.add([
            f"2 {sid}", f'"{name}"', '""',
            f'1 {x} {y} E E E 1 E 1 E N "_3DText" E N "_3DFace" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "4", '"sizePolicy" "Fixed Fixed"', '"layoutAlignment" "AlignCenter"',
            '"dashclr"N "_Transparent"', '"antiAliased" "0"',
            f"E E 0 1 1 0 1 E U  1 E {x} {y} {x + w} {y + 14}",
            f'0 0 0 "0s" 0 0 0 192 0 0  {x} {y} 1',
            *fonts(FONT_BOLD if bold else FONT_NORMAL),
            "0 3", *lang(de, en, ru),
        ])

    def textfield(self, name, x1, y1, x2, y2, length=1000, password=False):
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        fmt = "S" if password else "s"   # "S" hides the input (as vision/loginConfirmation.pnl)
        self.add([
            f"14 {sid}", f'"{name}"', '""',
            f'1 {x1 - 2} {y1} E E E 1 E 1 E N "_WindowText" E N "_Window" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            f'3 "{length}{fmt}" {length} 0 0 0 0 -1  E E E',
        ])

    def combobox(self, name, x1, y1, x2, y2, items=(), init=None, changed=None):
        """items: list of (de, en) tuples; the first one is preselected."""
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        rec = [
            f"22 {sid}", f'"{name}"', '""',
            f'1 {x1 - 2} {y1} E E E 1 E 1 E N "_WindowText" E N "_Window" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            str(len(items)),
        ]
        for i, (de, en) in enumerate(items):
            rec += ["3", *lang(de, en), "", "1" if i == 0 else "0"]
        rec.append("")
        # event slots: initialize, changed, (unused)
        rec.append(script(init) if init else "E")
        if init:
            rec.append("")
        rec.append(script(changed) if changed else "E")
        if changed:
            rec.append("")
        rec += ["E", " 0 0"]
        self.add(rec)

    def button(self, name, x1, y1, x2, y2, de, en, click, tooltip=("", "")):
        # layout of cmdNewDp in panels/para/poll_groups.pnl
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        self.add([
            f"13 {sid}", f'"{name}"', '""',
            f'1 {x1 + 7} {y1 + 12} E E E 1 E 1 E N "_ButtonText" E N "_Button" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(*tooltip),
            "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            "", "T ", "3", *lang(de, en),
            script(click),
            " E E E",
        ])

    def checkbox(self, name, x1, y1, x2, y2, de, en):
        # layout of chbPollGroupActive in panels/para/poll_groups.pnl
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        self.add([
            f"20 {sid}", f'"{name}"', '""',
            f'1 {x1 + 2} {y1 + 2} E E E 1 E 1 E N "_3DText" E N "_3DFace" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            "1", "T ", "3", *lang(de, en), "",
            "0 ", "3", *lang(""),
            "E E", "0 0 0 0 0", "1", "E E",
            "E" + script("main(int button, bool state)\n{\n}"),
        ])

    def spinbox(self, name, x1, y1, x2, y2, lo, hi):
        # layout of stastu in panels/para/poll_groups.pnl
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        self.add([
            f"21 {sid}", f'"{name}"', '""',
            f'1 {x1 - 2} {y1} E E E 1 E 1 E N "_WindowText" E N "_Window" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            "0", "", "E",
            script("main()\n{\n}"), "", "E", "",
            f"N {lo} {hi} 1 {lo} 1 1",
        ])

    def table(self, name, x1, y1, x2, y2, columns, clicked):
        """columns: (key, width, de, en); clicked: main(int row, string column, string value).
        Layout of tabClassFilter in panels/para/s7plus_displayclassfilter.pnl."""
        sid, ser = self.next()
        x1, x2 = x1 + self.ox, x2 + self.ox
        rec = [
            f"25 {sid}", f'"{name}"', '""',
            f'1 {x1} {y1} E E E 1 E 1 E N "_WindowText" E N "_Window" E E',
            " E E", f"{ser} 0 0 0 0 0", "E E E",
            *tip(),
            "3", '"acceptDrops" ""', '"sizePolicy" "Expanding Expanding"', '"layoutAlignment" "AlignCenter"',
            *fonts(FONT_NORMAL),
            f"0  {x1} {y1} {x2} {y2}",
            "E" + script(clicked),
        ]
        prefix = f" 1 0 1 {len(columns)} 1 "
        for i, (key, width, de, en) in enumerate(columns):
            # each column: its definition, header, tooltip; its width starts the next line
            rec += [f'{prefix}"{key}" 10 1 0 "s" 3', *lang(de, en), "E", "3", *lang(""), ""]
            prefix = f"{width} "
        rec += [prefix, '16 16 "" 1 3', *lang("#1"), "8 30", *fonts(FONT_NORMAL), "0 0 0 0 0 7", "1 0"]
        self.add(rec)


def header(name, width, height, dollars, init, terminate=None):
    lines = [
        "V 14", "3", *lang(name),
        f'PANEL,-1 -1 {width} {height} N "_3DFace" {len(dollars)}',
        *[f'"{d}"' for d in dollars],
        script(init),
    ]
    if terminate:
        lines += [" " + script(terminate), " E E E 1 -1 -1 0  0 0"]
    else:
        lines += [" E E E E 1 -1 -1 0  0 0"]
    lines += [
        '""0  1', "E E 4", '"CBRef" "0"', '"EClose" E', '"dpi" "96"', '"pdpi" "96"',
        "0 0 0", '""', "NC",
        "DISPLAY_LAYER, 1 0 1 0 1 0 1 0 1 0 1 0 1 0 1 0",
        "LAYER, 0 ", "1", "LANG:10000 6 Layer1",
    ]
    return lines


def layers_and_end(refs=()):
    """Other layers, then the reference panels (each a list of lines)."""
    out = []
    for n in range(1, 8):
        out += [f"LAYER, {n} ", "1", f"LANG:10000 6 Layer{n + 1}", "0"]
    for r in refs:
        out += r
    out.append("0")
    return out


def write(name, lines):
    path = PANELS / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {path} ({len(lines)} lines)")


# ============================================================ address panel

ADDRESS_INIT = """main()
{
  string dp;

  if ($1==\"\")
  {
    RootPanelOn(
      \"vision/MessageWarning\",
      paGetCatStr(\"para\",\"warning\"),
      makeDynString(paGetCatStr(\"para\",\"novision\")));
  }
  if (!paIsNative[paMyModuleId()])
  {
    dp = myUiDpName() + \".\";
    dpSetWait(dp+\"PanelOff.ModuleName:_original.._value\",myModuleName(),
          dp+\"PanelOff.PanelName:_original.._value\",\"address\");
  }
  paInitPanel($1,\":_address\");
}

""".replace('\\"', '"')

ADDRESS_TERM = """main()
{
  //IM 106997
  if(mappingHasKey(gMpLastDpLocked,myModuleName()) && (dpSubStr(gMpLastDpLocked[myModuleName()],DPSUB_SYS_DP_EL) == $1) && dpExists(dpSubStr($1,DPSUB_DP_EL)))
    dpSet($1+\":_lock._address._locked\",FALSE);
  if(mappingHasKey(gMpLastDpLocked,myModuleName()))
  {
    gMpLastDpLocked[myModuleName()] = \"\";
    mappingRemove(gMpLastDpLocked,myModuleName());
  }
}""".replace('\\"', '"')

TITLE = [
    "2 14", '"programname"', '""',
    '1 28 10 E E E 1 E 1 E N "_3DText" E N "_Transparent" E E',
    " E E", "14 0 0 0 0 0", "E E E", *tip(),
    "4", '"sizePolicy" "Fixed Fixed"', '"layoutAlignment" "AlignCenter"',
    '"dashclr"N "_Transparent"', '"antiAliased" "0"',
    "E E 0 1 1 0 1 E U  0 E 30 10 260 29",
    '0 2 0 "0s" 0 0 0 192 0 0  30 10 1',
    *fonts(FONT_TITLE),
    "0 3", *lang("Peripherie - MQTT (rsmqtt)", "Periphery - MQTT (rsmqtt)"),
    # z-order placeholders of the reference panels dpe.pnl / lock_unlock.pnl
    '1 66 0 "" 0', "0", '1 68 1 "" 0', "0",
]

LOWLEVEL = [
    "20 12", '"lowlevel"', '""',
    '1 438 128 E E E 1 E 1 E N "_3DText" E N "_3DFace" E E',
    " E E", "90 0 0 0 0 0", "E E E", *tip(),
    "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
    "0  438 128 581 156",
    "1", "T ", "3", *lang("Alt/Neu-Vergleich", "Old/new comparison", "Сравн. стар./нов."), "",
    "0 ", "3", *lang(""),
    "E E", "0 0 0 0 0", "1", "E E",
    # placeholder of para/buttons.pnl
    'EE1 70 2 "0" 0', "0",
]

DRIVER = [
    "21 35", '"Treiber"', '""',
    '1 496.6071428571428 63 E E E 1 E 1 E N "_WindowText" E N "_Window" E E',
    " E E", "91 0 0 0 0 0", "E E E", *tip(),
    "1", '"layoutAlignment" "AlignCenter"', *fonts(FONT_NORMAL),
    "0  498 63 597 89",
    "0", "", "E",
    script('#uses "driverSettings"\n\nmain()\n{\n  drvsCheckRunningDrvType("RSMQTT", this, $1);\n}'),
    "", "E", "", "N 1 256 1 1 1 1",
    # placeholder + position of para/refAddressActive.pnl
    '1 76 3 "" 0', '31 "transform" 0 0 1 0 1 -230 222', "0",
]

ADDRESS_REFS = [
    ['3 0 "" -1', '"layoutAlignment" "AlignCenter"', '"" ""',
     '"para/dpe.pnl" 308 10 T 0 1 0 1 2 0', "1", '"$DPE""$1"'],
    ['3 1 "" -1', '"layoutAlignment" "AlignCenter"', '"" ""',
     '"para/lock_unlock.pnl" 6 12 T 0 1 0 1 0 -3', "1", '"$1""$1"'],
    ['3 2 "" -1', '"layoutAlignment" "AlignCenter"', '"" ""',
     '"para/buttons.pnl" 220 440 T 0 1 0 1 -210 5', "0"],
    ['3 3 "Group5" -1', '"layoutAlignment" "AlignCenter"', '"" ""',
     '"para/refAddressActive.pnl" 10 450 T 36 1 0 1 330 -224', "0"],
    ['3 4 "PANEL_REF1" -1', '"layoutAlignment" "AlignNone"', '"" ""',
     '"para/addressVerifyButton.pnl" 246 440 T 44 1 0 1 -1 0', "1", '"$1""$1"'],
]

BROKER_INIT = """#uses "rsmqtt"

main()
{
  string current = this.text;
  this.editable = TRUE;
  this.items = rsmqttGetAllBrokers();
  this.text = current;
}"""

BROKER_CHANGED = """#uses "rsmqtt"

main()
{
  string broker = this.text;
  int drv = rsmqttGetBrokerDriver(broker);
  if ( drv > 0 )
    Treiber.text = drv;
  lblBrokerState.text = rsmqttBrokerStatusLine(broker);
  lblBrokerState.foreCol = "_3DText";
}"""

BROKERS_BUTTON = """#uses "rsmqtt"

main()
{
  dyn_float df;
  dyn_string ds;
  ChildPanelOnCentralModalReturn("para/rsmqtt_brokers.pnl", "MQTT broker (rsmqtt)",
                                 makeDynString("$broker:" + cmbBroker.text), df, ds);

  // the broker list may have changed; take over the broker chosen there
  string broker = cmbBroker.text;
  if ( dynlen(df) > 0 && df[1] == 1 && dynlen(ds) > 0 && ds[1] != "" )
    broker = ds[1];
  cmbBroker.items = rsmqttGetAllBrokers();
  cmbBroker.text = broker;
  int drv = rsmqttGetBrokerDriver(broker);
  if ( drv > 0 )
    Treiber.text = drv;
  lblBrokerState.text = rsmqttBrokerStatusLine(broker);
  lblBrokerState.foreCol = "_3DText";
}"""


def address_panel():
    p = Panel()
    p.add(TITLE)
    # row 1
    p.label("lblBroker", 20, 45, 60, "Broker")
    p.label("lblTopic", 230, 45, 60, "Topic")
    p.label("Text6", 500, 45, 100, "Treibernummer", "Driver number", "№ драйвера")
    # row 2
    p.label("lblDatatype", 20, 110, 120, "Datentyp", "Data type", "Тип данных")
    p.label("lblQos", 230, 110, 40, "QoS")
    p.label("lblRetain", 330, 110, 60, "Retain")
    # row 3
    p.label("lblDirection", 20, 180, 120, "Richtung", "Direction", "Режим")
    p.label("lblOptions", 230, 180, 160, "Weitere Optionen", "More options")
    # row 4: JSON
    p.label("lblJsonPath", 230, 258, 130, "JSON-Pfad", "JSON path")
    p.label("lblTsPath", 364, 258, 130, "Zeitstempel-Pfad", "Timestamp path")
    # hints
    p.label("lblHint1", 20, 318, 580,
            "Eingang: Topic darf + und # enthalten, $share/<Gruppe>/... für Shared Subscriptions.",
            "Input: the topic may contain + and #, $share/<group>/... for shared subscriptions.",
            bold=False)
    p.label("lblHint2", 20, 336, 580,
            "Topic $state = Verbindungszustand (1/0).  Optionen: nolocal; rap; rh=0|1|2",
            "Topic $state = connection state (1/0).  Options: nolocal; rap; rh=0|1|2",
            bold=False)
    p.label("lblHint3", 20, 354, 580,
            "JSON-Pfad z.B. data.temp, values[0], ['a.b']; leer = ganze Nutzdaten.",
            "JSON path e.g. data.temp, values[0], ['a.b']; empty = whole payload.",
            bold=False)
    p.label("lblHint4", 20, 372, 580,
            "Zeitstempel: ISO 8601 oder s/ms seit 1970.  dyn-Elemente: JSON-Array (dyn_blob: Base64).",
            "Timestamp: ISO 8601 or s/ms since 1970.  dyn elements: JSON array (dyn_blob: base64).",
            bold=False)
    p.label("lblHint5", 20, 390, 580,
            "Sparkplug B: Topic $spb/<Gruppe>/<Edge Node>[/<Gerät>], Option metric=<Name> oder online.",
            "Sparkplug B: topic $spb/<group>/<edge node>[/<device>], option metric=<name> or online.",
            bold=False)
    p.label("lblBrokerState", 20, 412, 580, "", bold=False)

    p.textfield("txtTopic", 228, 63, 492, 89)
    p.textfield("txtOptions", 228, 198, 492, 224, length=255)
    p.checkbox("chkJson", 18, 276, 222, 302, "JSON-Nutzdaten", "JSON payload")
    p.textfield("txtJsonPath", 228, 276, 358, 302, length=255)
    p.textfield("txtTsPath", 362, 276, 492, 302, length=255)

    p.combobox("trans_art", 18, 128, 222, 154, [
        ("Text (String)", "Text (string)"),
        ("Ganzzahl 32 Bit", "Integer 32 bit"),
        ("Ganzzahl 32 Bit vorzeichenlos", "Unsigned integer 32 bit"),
        ("Ganzzahl 64 Bit", "Integer 64 bit"),
        ("Ganzzahl 64 Bit vorzeichenlos", "Unsigned integer 64 bit"),
        ("Gleitkomma (double)", "Float (double)"),
        ("Bool", "Boolean"),
        ("Blob (Rohdaten)", "Blob (raw bytes)"),
    ])
    p.add(LOWLEVEL)
    p.add(DRIVER)
    p.combobox("einaus", 18, 198, 222, 224, [
        ("Ausgang (Publish)", "Output (publish)"),
        ("Eingang (Subscribe)", "Input (subscribe)"),
        ("Ein-/Ausgang", "Input/Output"),
    ])
    p.combobox("cmbQos", 228, 128, 322, 154, [
        ("Standard", "Default"), ("0", "0"), ("1", "1"), ("2", "2"),
    ])
    p.combobox("cmbRetain", 328, 128, 430, 154, [
        ("Standard", "Default"), ("Nein", "No"), ("Ja", "Yes"),
    ])
    p.button("cmdBrokers", 194, 62, 222, 90, "...", "...", BROKERS_BUTTON,
             tooltip=("Broker anlegen und bearbeiten", "Create and edit brokers"))
    p.combobox("cmbBroker", 18, 63, 190, 89, init=BROKER_INIT, changed=BROKER_CHANGED)

    lines = header("address_rsmqtt", 620, 480, ["$1"], ADDRESS_INIT, ADDRESS_TERM)
    lines += p.records
    # placeholder of para/addressVerifyButton.pnl, end of layer 0
    lines += ['1 77 4 "" 0', "0", "0"]
    lines += layers_and_end(ADDRESS_REFS)
    write("address_rsmqtt.pnl", lines)


# ============================================================= broker panel

BROKERS_PANEL_INIT = """#uses "rsmqtt"

main()
{
  string broker;
  if ( isDollarDefined("$broker") )
    broker = getDollarValue("$broker");
  rsmqttPanelInit(broker);
}"""

SELECT_CHANGED = """#uses "rsmqtt"

main()
{
  // an existing broker was picked (typing a new name keeps the form)
  if ( dynContains(this.items, this.text) > 0 )
    rsmqttPanelSelect(this.text);
  else
    rsmqttPanelUpdateState();
}"""

TABLE_CLICKED = """#uses "rsmqtt"

main(int row, string column, string value)
{
  rsmqttPanelTableClicked(row);
}"""

CLOSE_CLICKED = """#uses "rsmqtt"

main()
{
  if ( !rsmqttPanelConfirmDiscard() )
    return;
  string broker;
  getValue("cmbBroker", "text", broker);
  PanelOffReturn(makeDynFloat(1), makeDynString(broker));
}"""


def lib_click(call):
    """A button script that calls one rsmqtt.ctl function."""
    return '#uses "rsmqtt"\n\nmain()\n{\n  ' + call + ';\n}'


def broker_panel():
    TW = 560          # overview table on the left, the form right of it
    W = TW + 700
    p = Panel()
    p.label("lblTitle", 20, 12, 400, "MQTT-Broker (rsmqtt)", bold=True)

    p.ox = TW
    # selection + actions
    p.label("lblBrokerName", 20, 44, 200, "Broker (Name)", "Broker (name)")
    # editable: a new broker's name is typed here
    p.combobox("cmbBroker", 18, 62, 230, 88, init="main()\n{\n  this.editable = TRUE;\n}", changed=SELECT_CHANGED)
    # New only clears the form; re-setting the items would select the first
    # broker again and Save would overwrite it.
    p.button("cmdNew", 238, 61, 308, 89, "Neu", "New", lib_click("rsmqttPanelNew()"))
    p.button("cmdCopy", 314, 61, 400, 89, "Kopieren", "Copy", lib_click("rsmqttPanelCopy()"),
             tooltip=("Neuen Broker mit dieser Konfiguration anlegen", "New broker with this configuration"))
    p.button("cmdDelete", 406, 61, 486, 89, "Löschen", "Delete", lib_click("rsmqttPanelDelete()"))
    p.button("cmdReconnect", 492, 61, 622, 89, "Neu verbinden", "Reconnect", lib_click("rsmqttPanelReconnect()"))

    # live state
    p.label("lblStateCap", 20, 102, 70, "Zustand:", "State:")
    p.label("lblState", 92, 102, 590, "", bold=True)
    p.label("lblError", 92, 120, 590, "", bold=False)
    p.label("lblInfo", 92, 138, 590, "", bold=False)

    # connection
    y = 172
    p.label("lblSecConn", 20, y, 300, "Verbindung", "Connection")
    p.label("lblUri", 20, y + 22, 200, "Server-URI", "Server URI", bold=False)
    p.label("lblDrv", 482, y + 22, 100, "Treibernummer", "Driver number", bold=False)
    p.textfield("txtUri", 18, y + 40, 470, y + 66)
    p.spinbox("spinDriver", 480, y + 40, 560, y + 66, 1, 255)
    p.checkbox("chkActive", 576, y + 40, 690, y + 66, "Aktiv", "Active")

    y += 76
    p.label("lblUris", 20, y, 400, "Weitere Server-URIs (Failover, durch Leerzeichen getrennt)",
            "More server URIs (fail-over, separated by spaces)", bold=False)
    p.textfield("txtUris", 18, y + 18, 690, y + 44)

    y += 54
    p.label("lblClientId", 20, y, 200, "Client-ID (leer = automatisch)", "Client ID (empty = automatic)", bold=False)
    p.label("lblVersion", 300, y, 120, "MQTT-Version", "MQTT version", bold=False)
    p.label("lblKeepAlive", 440, y, 120, "Keep alive [s]", bold=False)
    p.textfield("txtClientId", 18, y + 18, 290, y + 44, length=255)
    p.combobox("cmbVersion", 298, y + 18, 430, y + 44, [
        ("Standard (5)", "Default (5)"), ("3.1", "3.1"), ("3.1.1", "3.1.1"), ("5", "5"),
    ])
    p.textfield("txtKeepAlive", 438, y + 18, 520, y + 44, length=10)
    p.checkbox("chkClean", 536, y + 18, 690, y + 44, "Clean Session", "Clean session")

    y += 54
    p.label("lblUser", 20, y, 200, "Benutzer", "User name", bold=False)
    p.label("lblPassword", 300, y, 200, "Passwort", "Password", bold=False)
    p.textfield("txtUser", 18, y + 18, 290, y + 44, length=255)
    p.textfield("txtPassword", 298, y + 18, 520, y + 44, length=255, password=True)

    # TLS
    y += 60
    p.label("lblSecTls", 20, y, 400, "TLS (für mqtts://, ssl://, wss://)", "TLS (for mqtts://, ssl://, wss://)")
    p.label("lblTrust", 20, y + 22, 200, "CA-Zertifikate (PEM)", "CA certificates (PEM)", bold=False)
    p.label("lblKeyStore", 240, y + 22, 200, "Client-Zertifikat (PEM)", "Client certificate (PEM)", bold=False)
    p.label("lblKey", 460, y + 22, 200, "Privater Schlüssel (PEM)", "Private key (PEM)", bold=False)
    p.textfield("txtTrust", 18, y + 40, 230, y + 66)
    p.textfield("txtKeyStore", 238, y + 40, 450, y + 66)
    p.textfield("txtKey", 458, y + 40, 690, y + 66)
    p.checkbox("chkVerify", 18, y + 72, 400, y + 96, "Hostname im Zertifikat prüfen", "Verify the host name in the certificate")

    # last will
    y += 108
    p.label("lblSecWill", 20, y, 400, "Letzter Wille (Last Will)", "Last will")
    p.label("lblWillTopic", 20, y + 22, 200, "Topic", bold=False)
    p.label("lblWillPayload", 300, y + 22, 200, "Payload", bold=False)
    p.label("lblWillQos", 520, y + 22, 60, "QoS", bold=False)
    p.textfield("txtWillTopic", 18, y + 40, 290, y + 66)
    p.textfield("txtWillPayload", 298, y + 40, 510, y + 66)
    p.combobox("cmbWillQos", 518, y + 40, 580, y + 66, [("0", "0"), ("1", "1"), ("2", "2")])
    p.checkbox("chkWillRetain", 590, y + 40, 690, y + 66, "Retain", "Retain")

    # address defaults
    y += 80
    p.label("lblSecDefaults", 20, y, 400, "Vorgaben für Adressen", "Defaults for addresses")
    p.label("lblSubQos", 20, y + 22, 130, "Subscribe-QoS", "Subscribe QoS", bold=False)
    p.label("lblPubQos", 160, y + 22, 130, "Publish-QoS", "Publish QoS", bold=False)
    p.combobox("cmbSubQos", 18, y + 40, 150, y + 66, [("0", "0"), ("1", "1"), ("2", "2")])
    p.combobox("cmbPubQos", 158, y + 40, 290, y + 66, [("0", "0"), ("1", "1"), ("2", "2")])
    p.checkbox("chkPubRetain", 300, y + 40, 500, y + 66, "Publish mit Retain", "Publish retained")

    # Sparkplug B host application (the options sparkplug.hostId / redu.sparkplug.hostId)
    y += 80
    p.label("lblSecSpb", 20, y, 600, "Sparkplug B: primäre Host Application (leer = keine)",
            "Sparkplug B: primary host application (empty = none)")
    p.label("lblSpbHost", 20, y + 22, 260, "Host-ID", "Host ID", bold=False)
    p.label("lblSpbHost2", 300, y + 22, 300, "Host-ID des 2. redundanten Hosts", "Host ID of the 2nd redundant host",
            bold=False)
    p.textfield("txtSpbHost", 18, y + 40, 290, y + 66, length=255)
    p.textfield("txtSpbHost2", 298, y + 40, 570, y + 66, length=255)
    p.label("lblSpbState", 20, y + 72, 660, "", bold=False)

    y += 98
    p.label("lblOptions", 20, y, 660,
            "Weitere rsmqtt-Optionen (name=wert; ...), z.B. sessionExpiryInterval=3600; ssl.alpn=mqtt",
            "More rsmqtt options (name=value; ...), e.g. sessionExpiryInterval=3600; ssl.alpn=mqtt", bold=False)
    p.textfield("txtOptions", 18, y + 18, 470, y + 44)
    # picking an option adds it with an example value to txtOptions
    p.combobox("cmbAddOption", 478, y + 18, 690, y + 44,
               [("(Option hinzufügen ...)", "(add an option ...)")] + [(o, o) for o in PANEL_OPTIONS],
               changed=lib_click("rsmqttPanelAddOption()"))

    y += 56
    p.label("lblMessage", 20, y, 660, "", bold=True)

    y += 26
    p.button("cmdSave", 470, y, 580, y + 30, "Speichern", "Save", lib_click("rsmqttPanelSave()"))
    p.button("cmdClose", 586, y, 690, y + 30, "Schließen", "Close", CLOSE_CLICKED)

    # panel state, hidden by rsmqttPanelInit: the broker shown, the form as
    # last shown or saved (unsaved changes), the broker a copy was made from
    p.label("lblShown", 20, y + 4, 100, "", bold=False)
    p.label("lblSnapshot", 20, y + 4, 100, "", bold=False)
    p.label("lblCopyFrom", 20, y + 4, 100, "", bold=False)

    # overview of all brokers: updated every second, a click shows the broker
    p.ox = 0
    p.label("lblOverview", 20, 44, 400, "Alle Broker", "All brokers")
    p.table("tabBrokers", 18, 62, TW - 10, y + 30, [
        ("broker", 110, "Broker", "Broker"),
        ("drv", 50, "Treiber", "Driver"),
        ("state", 130, "Zustand", "State"),
        ("io", 90, "Ein / Aus", "In / out"),
        ("error", 150, "Letzter Fehler", "Last error"),
    ], TABLE_CLICKED)

    height = y + 44
    lines = header("rsmqtt_brokers", W, height, ["$broker"], BROKERS_PANEL_INIT)
    lines += p.records
    lines += ["0"]   # end of layer 0
    lines += layers_and_end()
    write("rsmqtt_brokers.pnl", lines)


if __name__ == "__main__":
    check_option_list()
    address_panel()
    broker_panel()
