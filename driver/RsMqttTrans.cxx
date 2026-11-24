#include "RsMqttTrans.hxx"
#include "RsMqttTransformationType.hxx"
#include "RsMqttResources.hxx"
#include "RsMqttJson.hxx"

#include <BitVar.hxx>
#include <BlobVar.hxx>
#include <DynVar.hxx>
#include <ErrHdl.hxx>
#include <FloatVar.hxx>
#include <IntegerVar.hxx>
#include <LongVar.hxx>
#include <TextVar.hxx>
#include <UIntegerVar.hxx>
#include <ULongVar.hxx>

#include <charconv>
#include <cmath>
#include <cstring>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

namespace T = RsMqttTransformationType;

namespace
{
  TransformationType normalize(TransformationType t)
  {
    return (t == T::Undefined || t == 0) ? T::String : t;
  }

  void convError(const char *func, const CharString &text)
  {
    ErrHdl::error(ErrClass::PRIO_WARNING, ErrClass::ERR_PARAM, ErrClass::UNEXPECTEDSTATE,
                  "RsMqttTrans", func, text);
  }

  template <class N> std::string numToString(N v)
  {
    char buf[64];
    auto r = std::to_chars(buf, buf + sizeof(buf), v);
    return std::string(buf, r.ptr);
  }

  std::string shown(std::string_view payload)
  {
    std::string s(payload.substr(0, 64));
    return "'" + s + (payload.size() > 64 ? "...'" : "'");
  }
}

//----------------------------------------------------------------------------

RsMqttTrans::RsMqttTrans(TransformationType t, bool d) : type(normalize(t)), dyn(d) {}

bool RsMqttTrans::isSupported(TransformationType t)
{
  t = normalize(t);
  return t >= T::String && t <= T::Blob;
}

bool RsMqttTrans::isSupportedDyn(TransformationType t)
{
  t = normalize(t);
  return t >= T::String && t <= T::Bool;
}

TransformationType RsMqttTrans::isA() const { return type; }

int RsMqttTrans::itemSize() const { return (int) (RsMqttResources::getMaxPayload() + 4); }

Transformation *RsMqttTrans::clone() const { return new RsMqttTrans(type, dyn); }

VariableType RsMqttTrans::itemType() const
{
  switch ( type )
  {
    case T::Int32:  return INTEGER_VAR;
    case T::UInt32: return UINTEGER_VAR;
    case T::Int64:  return LONG_VAR;
    case T::UInt64: return ULONG_VAR;
    case T::Float:  return FLOAT_VAR;
    case T::Bool:   return BIT_VAR;
    case T::Blob:   return BLOB_VAR;
    default:        return TEXT_VAR;
  }
}

VariableType RsMqttTrans::getVariableType() const
{
  return dyn ? DynVar::getDynType(itemType()) : itemType();
}

//----------------------------------------------------------------------------

bool RsMqttTrans::itemToText(const Variable &var, bool json, std::string &text) const
{
  if ( var.isA() != itemType() )
  {
    convError("toPeriph", CharString("unexpected variable type ") + CharString((int) var.isA()));
    return false;
  }
  switch ( type )
  {
    case T::Int32:  text = numToString((long long) static_cast<const IntegerVar &>(var).getValue()); break;
    case T::UInt32: text = numToString((unsigned long long) static_cast<const UIntegerVar &>(var).getValue()); break;
    case T::Int64:  text = numToString((long long) static_cast<const LongVar &>(var).getValue()); break;
    case T::UInt64: text = numToString((unsigned long long) static_cast<const ULongVar &>(var).getValue()); break;
    case T::Float:
    {
      double d = static_cast<const FloatVar &>(var).getValue();
      if ( !std::isfinite(d) )
      {
        convError("toPeriph", "cannot publish a non-finite float value");
        return false;
      }
      text = numToString(d);  // shortest round-trip representation
      break;
    }
    case T::Bool:   text = static_cast<const BitVar &>(var).getValue() ? "true" : "false"; break;
    default:
    {
      const CharString &s = static_cast<const TextVar &>(var).getString();
      std::string_view v((const char *) s, s.len());
      text = json ? RsMqttJson::quote(v) : std::string(v);
      break;
    }
  }
  return true;
}

PVSSboolean RsMqttTrans::toPeriph(PVSSchar *dataPtr, PVSSushort len, const Variable &var, const PVSSushort subix) const
{
  return toPeriph(dataPtr, (PVSSulong) len, var, (PVSSulong) subix);
}

PVSSboolean RsMqttTrans::toPeriph(PVSSchar *dataPtr, PVSSulong len, const Variable &var, const PVSSulong) const
{
  std::string text;
  const PVSSchar *raw = nullptr;
  size_t rawLen = 0;

  if ( var.isA() != getVariableType() )
  {
    convError("toPeriph", CharString("unexpected variable type ") + CharString((int) var.isA()));
    return PVSS_FALSE;
  }

  if ( dyn )
  {
    // a JSON array, one element per item
    const DynVar &d = static_cast<const DynVar &>(var);
    text = "[";
    for (unsigned int i = 0; i < d.getNumberOfItems(); i++)
    {
      const Variable *item = d.getAt(i);
      std::string t;
      if ( !item || !itemToText(*item, true, t) )
        return PVSS_FALSE;
      if ( i ) text += ',';
      text += t;
    }
    text += ']';
  }
  else if ( type == T::Blob )
  {
    const Blob &b = static_cast<const BlobVar &>(var).getValue();
    raw = reinterpret_cast<const PVSSchar *>(b.getData());
    rawLen = b.getLen();
  }
  else if ( !itemToText(var, false, text) )
    return PVSS_FALSE;

  if ( !raw )
  {
    raw = reinterpret_cast<const PVSSchar *>(text.data());
    rawLen = text.size();
  }

  if ( rawLen + 4 > len )
  {
    convError("toPeriph", CharString("payload of ") + CharString((int) rawLen) +
              " bytes exceeds maxPayload " + CharString((int) RsMqttResources::getMaxPayload()));
    return PVSS_FALSE;
  }

  uint32_t n = (uint32_t) rawLen;
  dataPtr[0] = (PVSSchar) (n & 0xff);
  dataPtr[1] = (PVSSchar) ((n >> 8) & 0xff);
  dataPtr[2] = (PVSSchar) ((n >> 16) & 0xff);
  dataPtr[3] = (PVSSchar) ((n >> 24) & 0xff);
  if ( rawLen )
    memcpy(dataPtr + 4, raw, rawLen);
  return PVSS_TRUE;
}

//----------------------------------------------------------------------------

Variable *RsMqttTrans::textToItem(std::string_view text, bool jsonString, std::string &why) const
{
  if ( type == T::String )
    return new TextVar(text.data(), text.size());
  if ( type == T::Blob )
    return new BlobVar(reinterpret_cast<PVSSuchar *>(const_cast<char *>(text.data())), (PVSSulong) text.size(), true);  // copy

  // numbers and bool: crates/rsmqtt-wincc/src/values.rs
  rsmqtt_wincc_number n{};
  if ( !rsmqtt::wincc::values::parseNumber((int) type, text, jsonString, n, why) )
    return nullptr;
  switch ( type )
  {
    case T::Int32:  return new IntegerVar((int32_t) n.i);
    case T::UInt32: return new UIntegerVar((uint32_t) n.u);
    case T::Int64:  return new LongVar((int64_t) n.i);
    case T::UInt64: return new ULongVar((uint64_t) n.u);
    case T::Float:  return new FloatVar(n.f);
    case T::Bool:   return new BitVar(n.b ? PVSS_TRUE : PVSS_FALSE);
    default:        break;
  }
  why = "unsupported data type " + std::to_string((int) type);
  return nullptr;
}

VariablePtr RsMqttTrans::toDynVar(std::string_view payload) const
{
  // a JSON array, one value or nothing (rsmqtt-wincc, json::dyn_items)
  std::vector<std::pair<std::string, bool>> items;
  std::string err;
  if ( !RsMqttJson::dynItems(payload, items, err) )
  {
    convError("toVar", CharString(err.c_str()) + " in " + shown(payload).c_str());
    return nullptr;
  }
  std::unique_ptr<DynVar> d(new DynVar(itemType()));
  for (size_t i = 0; i < items.size(); i++)
  {
    std::string why;
    std::unique_ptr<Variable> item(textToItem(items[i].first, items[i].second, why));
    if ( !item )
    {
      convError("toVar", CharString("item ") + CharString((int) i) + ": " + why.c_str());
      return nullptr;
    }
    d->append(*item);
  }
  return d.release();
}

VariablePtr RsMqttTrans::toVar(const PVSSchar *data, const PVSSushort dlen, const PVSSushort subix) const
{
  return toVar(data, (PVSSulong) dlen, (PVSSulong) subix);
}

VariablePtr RsMqttTrans::toVar(const PVSSchar *data, const PVSSulong dlen, const PVSSulong) const
{
  std::string_view payload(reinterpret_cast<const char *>(data), data ? dlen : 0);

  if ( dyn )
    return toDynVar(payload);

  std::string why;
  Variable *var = textToItem(payload, false, why);
  if ( !var )
    convError("toVar", why.c_str());
  return var;
}
