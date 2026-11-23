#include "RsMqttAddress.hxx"

#include <cctype>

namespace
{
  std::string_view trim(std::string_view s)
  {
    while ( !s.empty() && std::isspace(static_cast<unsigned char>(s.front())) ) s.remove_prefix(1);
    while ( !s.empty() && std::isspace(static_cast<unsigned char>(s.back())) ) s.remove_suffix(1);
    return s;
  }

  bool ieq(std::string_view a, std::string_view b)
  {
    if ( a.size() != b.size() ) return false;
    for (size_t i = 0; i < a.size(); i++)
      if ( std::tolower(static_cast<unsigned char>(a[i])) != std::tolower(static_cast<unsigned char>(b[i])) )
        return false;
    return true;
  }

  bool parseBool(std::string_view v, int &out)
  {
    if ( v == "1" || ieq(v, "true") || ieq(v, "yes") || ieq(v, "on") )  { out = 1; return true; }
    if ( v == "0" || ieq(v, "false") || ieq(v, "no") || ieq(v, "off") ) { out = 0; return true; }
    return false;
  }

  // Removes "$share/<group>/" from a shared subscription filter.
  std::string_view stripShare(std::string_view filter)
  {
    constexpr std::string_view share = "$share/";
    if ( filter.substr(0, share.size()) != share ) return filter;
    std::string_view rest = filter.substr(share.size());
    size_t slash = rest.find('/');
    return slash == std::string_view::npos ? std::string_view() : rest.substr(slash + 1);
  }
}

bool RsMqttAddress::hasWildcards(std::string_view filter)
{
  return filter.find_first_of("+#") != std::string_view::npos;
}

bool RsMqttAddress::isWildcardFilter(std::string_view filter)
{
  return hasWildcards(filter) || filter.substr(0, 7) == "$share/";
}

bool RsMqttAddress::topicMatches(std::string_view filter, std::string_view topic)
{
  filter = stripShare(filter);
  if ( filter.empty() ) return false;

  // [MQTT-4.7.2-1] wildcards at the first level don't match $-topics
  if ( !topic.empty() && topic.front() == '$' && (filter.front() == '+' || filter.front() == '#') )
    return false;

  size_t f = 0, t = 0;
  for (;;)
  {
    size_t fe = filter.find('/', f);
    std::string_view flevel = filter.substr(f, fe == std::string_view::npos ? std::string_view::npos : fe - f);

    if ( flevel == "#" )
      return true;  // matches this level and everything below, incl. the parent

    if ( t > topic.size() )
    {
      // topic exhausted: only "a/#" matching "a" is left, handled above
      return false;
    }

    size_t te = topic.find('/', t);
    std::string_view tlevel = topic.substr(t, te == std::string_view::npos ? std::string_view::npos : te - t);

    if ( flevel != "+" && flevel != tlevel )
      return false;

    bool fLast = (fe == std::string_view::npos);
    bool tLast = (te == std::string_view::npos);

    if ( fLast && tLast ) return true;
    if ( fLast ) return false;   // topic has more levels
    if ( tLast )
    {
      // filter has more levels; "a/#" matches "a"
      return filter.substr(fe + 1) == "#";
    }
    f = fe + 1;
    t = te + 1;
  }
}

bool RsMqttAddress::parse(std::string_view text, RsMqttAddress &out, std::string &err)
{
  out = RsMqttAddress();
  text = trim(text);

  size_t colon = text.find(':');
  if ( colon == std::string_view::npos || colon == 0 )
  {
    err = "address must be '<connection>:<topic>'";
    return false;
  }
  out.connection = std::string(trim(text.substr(0, colon)));
  std::string_view rest = text.substr(colon + 1);

  size_t semi = rest.find(';');
  std::string_view topic = rest.substr(0, semi);
  if ( topic.empty() )
  {
    err = "empty topic";
    return false;
  }
  out.topic = std::string(topic);
  out.isState = (topic == "$state");

  // A topic filter must not have '#' anywhere but as last level, '+' only as
  // a whole level.
  for (size_t i = 0; i < topic.size(); i++)
  {
    char c = topic[i];
    if ( c == '#' && (i + 1 != topic.size() || (i > 0 && topic[i - 1] != '/')) )
    {
      err = "'#' must be the last level of a topic filter";
      return false;
    }
    if ( c == '+' && ((i > 0 && topic[i - 1] != '/') || (i + 1 < topic.size() && topic[i + 1] != '/')) )
    {
      err = "'+' must occupy a whole topic level";
      return false;
    }
  }

  while ( semi != std::string_view::npos )
  {
    rest = rest.substr(semi + 1);
    semi = rest.find(';');
    std::string_view opt = trim(rest.substr(0, semi));
    if ( opt.empty() ) continue;

    size_t eq = opt.find('=');
    std::string_view name = trim(opt.substr(0, eq));
    std::string_view val = eq == std::string_view::npos ? std::string_view() : trim(opt.substr(eq + 1));

    if ( ieq(name, "qos") )
    {
      if ( val.size() != 1 || val[0] < '0' || val[0] > '2' )
      {
        err = "qos must be 0, 1 or 2";
        return false;
      }
      out.qos = val[0] - '0';
    }
    else if ( ieq(name, "retain") )
    {
      int b = 1;
      if ( !val.empty() && !parseBool(val, b) )
      {
        err = "retain must be 0 or 1";
        return false;
      }
      out.retain = b;
    }
    else if ( ieq(name, "nolocal") || ieq(name, "nl") )
      out.subFlags |= 0x04;
    else if ( ieq(name, "rap") )
      out.subFlags |= 0x08;
    else if ( ieq(name, "rh") )
    {
      if ( val.size() != 1 || val[0] < '0' || val[0] > '2' )
      {
        err = "rh must be 0, 1 or 2";
        return false;
      }
      out.subFlags = (out.subFlags & ~0x30) | ((val[0] - '0') << 4);
    }
    else if ( ieq(name, "json") )
    {
      std::string perr;
      if ( !RsMqttJson::parsePath(val, out.jsonPath, perr) )
      {
        err = perr;
        return false;
      }
      out.json = true;
    }
    else if ( ieq(name, "ts") )
    {
      std::string perr;
      if ( val.empty() )
      {
        err = "ts needs a JSON path, e.g. ts=time";
        return false;
      }
      if ( !RsMqttJson::parsePath(val, out.tsPath, perr) )
      {
        err = "ts: " + perr;
        return false;
      }
      if ( out.tsPath.empty() )
      {
        err = "ts needs a JSON path below the top level, e.g. ts=time";
        return false;
      }
      out.hasTs = true;
    }
    else
    {
      err = "unknown address option '" + std::string(name) + "'";
      return false;
    }
  }

  if ( out.hasTs && !out.json )
  {
    err = "ts needs the json option";
    return false;
  }
  if ( out.json && out.isState )
  {
    err = "$state has no JSON payload";
    return false;
  }
  if ( out.hasTs )
  {
    // the published object must hold both: neither path may contain the other
    std::string berr, ignored;
    std::vector<std::pair<const RsMqttJson::Path *, std::string>> m = {{&out.jsonPath, "0"}, {&out.tsPath, "0"}};
    if ( !RsMqttJson::build(m, ignored, berr) )
    {
      err = "json and ts paths collide";
      return false;
    }
  }
  return true;
}
