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

#include <cctype>
#include <charconv>
#include <cmath>
#include <cstring>
#include <limits>
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

  std::string_view trim(std::string_view s)
  {
    while ( !s.empty() && std::isspace(static_cast<unsigned char>(s.front())) ) s.remove_prefix(1);
    while ( !s.empty() && std::isspace(static_cast<unsigned char>(s.back())) ) s.remove_suffix(1);
    // tolerate JSON strings like "\"21.5\""
    if ( s.size() >= 2 && s.front() == '"' && s.back() == '"' ) s = s.substr(1, s.size() - 2);
    return s;
  }

  bool ieq(std::string_view a, const char *b)
  {
    size_t n = strlen(b);
    if ( a.size() != n ) return false;
    for (size_t i = 0; i < n; i++)
      if ( std::tolower(static_cast<unsigned char>(a[i])) != b[i] ) return false;
    return true;
  }

  bool parseBool(std::string_view s, bool &out)
  {
    if ( ieq(s, "true") || ieq(s, "on") || ieq(s, "yes") ) { out = true; return true; }
    if ( ieq(s, "false") || ieq(s, "off") || ieq(s, "no") ) { out = false; return true; }
    double d = 0;
    auto r = std::from_chars(s.data(), s.data() + s.size(), d);
    if ( r.ec == std::errc() && r.ptr == s.data() + s.size() ) { out = (d != 0.0); return true; }
    return false;
  }

  bool parseDouble(std::string_view s, double &out)
  {
    bool b;
    if ( ieq(s, "true") || ieq(s, "false") ) { parseBool(s, b); out = b ? 1.0 : 0.0; return true; }
    if ( !s.empty() && s.front() == '+' ) s.remove_prefix(1);
    auto r = std::from_chars(s.data(), s.data() + s.size(), out);
    return r.ec == std::errc() && r.ptr == s.data() + s.size();
  }

  // Integer parse; accepts integral floating point text like "12.0" or "1e3".
  template <class I> bool parseInt(std::string_view s, I &out)
  {
    std::string_view t = s;
    if ( !t.empty() && t.front() == '+' ) t.remove_prefix(1);
    auto r = std::from_chars(t.data(), t.data() + t.size(), out);
    if ( r.ec == std::errc() && r.ptr == t.data() + t.size() ) return true;
    double d;
    if ( !parseDouble(s, d) || !std::isfinite(d) || std::floor(d) != d ) return false;
    // parenthesized: windows.h may define a max() macro
    if ( d < (double) (std::numeric_limits<I>::lowest)() || d > (double) (std::numeric_limits<I>::max)() ) return false;
    out = static_cast<I>(d);
    return true;
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

  std::string_view s = jsonString ? text : trim(text);
  if ( jsonString )
    while ( !s.empty() && std::isspace(static_cast<unsigned char>(s.front())) ) s.remove_prefix(1);
  switch ( type )
  {
    case T::Int32:  { int32_t v;  if ( parseInt(s, v) ) return new IntegerVar(v);  break; }
    case T::UInt32: { uint32_t v; if ( parseInt(s, v) ) return new UIntegerVar(v); break; }
    case T::Int64:  { int64_t v;  if ( parseInt(s, v) ) return new LongVar(v);     break; }
    case T::UInt64: { uint64_t v; if ( parseInt(s, v) ) return new ULongVar(v);    break; }
    case T::Float:  { double v;   if ( parseDouble(s, v) ) return new FloatVar(v); break; }
    case T::Bool:   { bool v;     if ( parseBool(s, v) ) return new BitVar(v ? PVSS_TRUE : PVSS_FALSE); break; }
    default: break;
  }
  why = "cannot convert " + shown(text) + " to the configured data type " + std::to_string((int) type);
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
