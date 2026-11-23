"""Sparkplug B edge node for the WinCC OA option tests (no third-party modules).

    python tests/broker/spb_edge.py pub <topic> <seq|-> <metric> [<metric> ...]
    python tests/broker/spb_edge.py decode <file in the broker container>

<metric> is  name:Type=value  or  name:Type=value@alias  or  @alias:Type=value
(alias only, as in NDATA). Type is a Sparkplug data type name (Int32, Double,
Boolean, String, FloatArray, ...); array values are JSON arrays, e.g.
"vals:FloatArray=[1.5,2]". The payload is encoded with sparkplug_b.proto and
published with mosquitto_pub inside the broker container (QoS 0).

decode prints the metrics of a captured payload (e.g. an NCMD / DCMD the
driver sent, captured with mosquitto_sub -N) as JSON.
"""

import json
import os
import struct
import subprocess
import sys
import tempfile
import time

CONTAINER = os.environ.get("RSMQTT_BROKER_CONTAINER", "rsmqtt-mosq-full")

TYPES = ["Unknown", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
         "Float", "Double", "Boolean", "String", "DateTime", "Text", "UUID", "DataSet", "Bytes", "File",
         "Template", "PropertySet", "PropertySetList", "Int8Array", "Int16Array", "Int32Array", "Int64Array",
         "UInt8Array", "UInt16Array", "UInt32Array", "UInt64Array", "FloatArray", "DoubleArray",
         "BooleanArray", "StringArray", "DateTimeArray"]
ARRAY_FMT = {"Int8Array": "b", "Int16Array": "h", "Int32Array": "i", "Int64Array": "q",
             "UInt8Array": "B", "UInt16Array": "H", "UInt32Array": "I", "UInt64Array": "Q",
             "FloatArray": "f", "DoubleArray": "d", "DateTimeArray": "q"}


def varint(v):
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def field_varint(no, v):
    return varint(no << 3) + varint(v)


def field_bytes(no, b):
    return varint((no << 3) | 2) + varint(len(b)) + b


def encode_metric(spec, ts):
    name_part, value = spec.split("=", 1)
    alias = None
    if "@" in value and not value.startswith("[") and not name_part.startswith("@"):
        value, alias = value.rsplit("@", 1)
    name, type_name = name_part.rsplit(":", 1)
    if name.startswith("@"):
        alias, name = name[1:], ""
    t = TYPES.index(type_name)
    out = b""
    if name:
        out += field_bytes(1, name.encode("utf-8"))
    if alias is not None:
        out += field_varint(2, int(alias))
    out += field_varint(3, ts) + field_varint(4, t)
    if type_name in ("Int8", "Int16", "Int32"):
        out += field_varint(10, int(value) & 0xFFFFFFFF)
    elif type_name in ("UInt8", "UInt16"):
        out += field_varint(10, int(value))
    elif type_name in ("Int64", "UInt32", "UInt64", "DateTime"):
        out += field_varint(11, int(value) & 0xFFFFFFFFFFFFFFFF)
    elif type_name == "Float":
        out += varint((12 << 3) | 5) + struct.pack("<f", float(value))
    elif type_name == "Double":
        out += varint((13 << 3) | 1) + struct.pack("<d", float(value))
    elif type_name == "Boolean":
        out += field_varint(14, 1 if value.lower() in ("1", "true") else 0)
    elif type_name in ("String", "Text", "UUID"):
        out += field_bytes(15, value.encode("utf-8"))
    elif type_name in ("Bytes", "File"):
        out += field_bytes(16, value.encode("utf-8"))
    elif type_name in ARRAY_FMT:
        items = json.loads(value)
        out += field_bytes(16, struct.pack("<%d%s" % (len(items), ARRAY_FMT[type_name]), *items))
    elif type_name == "BooleanArray":
        items = json.loads(value)
        bits = bytearray((len(items) + 7) // 8)
        for k, b in enumerate(items):
            if b:
                bits[k // 8] |= 0x80 >> (k % 8)
        out += field_bytes(16, struct.pack("<I", len(items)) + bytes(bits))
    elif type_name == "StringArray":
        out += field_bytes(16, b"".join(s.encode("utf-8") + b"\0" for s in json.loads(value)))
    else:
        raise SystemExit("unsupported type " + type_name)
    return out


def publish(topic, seq, specs):
    ts = int(time.time() * 1000)
    payload = field_varint(1, ts)
    for s in specs:
        payload += field_bytes(2, encode_metric(s, ts))
    if seq != "-":
        payload += field_varint(3, int(seq))
    with tempfile.NamedTemporaryFile(delete=False, suffix=".bin") as f:
        f.write(payload)
        path = f.name
    try:
        subprocess.run(["docker", "cp", path, CONTAINER + ":/tmp/spb.bin"], check=True, capture_output=True)
        subprocess.run(["docker", "exec", CONTAINER, "mosquitto_pub", "-p", "1883", "-t", topic, "-f", "/tmp/spb.bin"],
                       check=True, capture_output=True)
    finally:
        os.unlink(path)
    print("published %s (%d bytes)" % (topic, len(payload)))


class Reader:
    def __init__(self, b):
        self.b, self.i = b, 0

    def varint(self):
        v, shift = 0, 0
        while True:
            c = self.b[self.i]
            self.i += 1
            v |= (c & 0x7F) << shift
            if not c & 0x80:
                return v
            shift += 7

    def take(self, n):
        out = self.b[self.i:self.i + n]
        self.i += n
        return out


def decode_metric(b):
    r, m = Reader(b), {}
    while r.i < len(b):
        key = r.varint()
        no, wt = key >> 3, key & 7
        if wt == 0:
            v = r.varint()
        elif wt == 1:
            v = r.take(8)
        elif wt == 5:
            v = r.take(4)
        else:
            v = r.take(r.varint())
        if no == 1:
            m["name"] = v.decode("utf-8")
        elif no == 2:
            m["alias"] = v
        elif no == 4:
            m["datatype"] = TYPES[v] if v < len(TYPES) else v
        elif no == 10:
            m["value"] = struct.unpack("<i", struct.pack("<I", v))[0]
        elif no == 11:
            m["value"] = v
        elif no == 12:
            m["value"] = struct.unpack("<f", v)[0]
        elif no == 13:
            m["value"] = struct.unpack("<d", v)[0]
        elif no == 14:
            m["value"] = bool(v)
        elif no == 15:
            m["value"] = v.decode("utf-8")
        elif no == 16:
            m["bytes"] = v.hex()
    return m


def decode(path):
    data = subprocess.run(["docker", "exec", CONTAINER, "cat", path], check=True, capture_output=True).stdout
    r, metrics = Reader(data), []
    while r.i < len(data):
        key = r.varint()
        no, wt = key >> 3, key & 7
        if wt == 0:
            r.varint()
        elif wt == 2:
            b = r.take(r.varint())
            if no == 2:
                metrics.append(decode_metric(b))
        else:
            r.take(8 if wt == 1 else 4)
    print(json.dumps(metrics, sort_keys=True))


def main():
    if len(sys.argv) >= 4 and sys.argv[1] == "pub":
        publish(sys.argv[2], sys.argv[3], sys.argv[4:])
    elif len(sys.argv) == 3 and sys.argv[1] == "decode":
        decode(sys.argv[2])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
