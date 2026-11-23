#include "RsMqttJson.hxx"

#include <cctype>
#include <charconv>
#include <cmath>
#include <cstdio>
#include <memory>

namespace RsMqttJson
{
namespace
{
  constexpr int maxDepth = 256;

  // ------------------------------------------------------------------ UTF-8

  void appendUtf8(std::string &out, unsigned long cp)
  {
    if ( cp < 0x80 )
      out += (char) cp;
    else if ( cp < 0x800 )
    {
      out += (char) (0xC0 | (cp >> 6));
      out += (char) (0x80 | (cp & 0x3F));
    }
    else if ( cp < 0x10000 )
    {
      out += (char) (0xE0 | (cp >> 12));
      out += (char) (0x80 | ((cp >> 6) & 0x3F));
      out += (char) (0x80 | (cp & 0x3F));
    }
    else
    {
      out += (char) (0xF0 | (cp >> 18));
      out += (char) (0x80 | ((cp >> 12) & 0x3F));
      out += (char) (0x80 | ((cp >> 6) & 0x3F));
      out += (char) (0x80 | (cp & 0x3F));
    }
  }

  /// Length of the valid UTF-8 sequence at s[i], 0 if invalid.
  size_t utf8Len(std::string_view s, size_t i)
  {
    unsigned char c = (unsigned char) s[i];
    size_t n;
    unsigned long cp;
    if ( c < 0x80 ) return 1;
    else if ( (c & 0xE0) == 0xC0 ) { n = 2; cp = c & 0x1F; }
    else if ( (c & 0xF0) == 0xE0 ) { n = 3; cp = c & 0x0F; }
    else if ( (c & 0xF8) == 0xF0 ) { n = 4; cp = c & 0x07; }
    else return 0;
    if ( i + n > s.size() ) return 0;
    for (size_t k = 1; k < n; k++)
    {
      unsigned char cc = (unsigned char) s[i + k];
      if ( (cc & 0xC0) != 0x80 ) return 0;
      cp = (cp << 6) | (cc & 0x3F);
    }
    // overlong forms, surrogates, beyond U+10FFFF
    if ( (n == 2 && cp < 0x80) || (n == 3 && cp < 0x800) || (n == 4 && cp < 0x10000) ) return 0;
    if ( (cp >= 0xD800 && cp <= 0xDFFF) || cp > 0x10FFFF ) return 0;
    return n;
  }

  // ----------------------------------------------------------------- parser

  struct Parser
  {
    std::string_view s;
    size_t i = 0;
    std::string err;

    bool fail(const char *what)
    {
      if ( err.empty() )
        err = std::string(what) + " at offset " + std::to_string(i);
      return false;
    }

    void ws()
    {
      while ( i < s.size() && (s[i] == ' ' || s[i] == '\t' || s[i] == '\n' || s[i] == '\r') ) i++;
    }

    bool hex4(unsigned long &v)
    {
      if ( i + 4 > s.size() ) return fail("short \\u escape");
      v = 0;
      for (int k = 0; k < 4; k++)
      {
        char c = s[i++];
        v <<= 4;
        if ( c >= '0' && c <= '9' ) v |= (unsigned long) (c - '0');
        else if ( c >= 'a' && c <= 'f' ) v |= (unsigned long) (c - 'a' + 10);
        else if ( c >= 'A' && c <= 'F' ) v |= (unsigned long) (c - 'A' + 10);
        else return fail("bad \\u escape");
      }
      return true;
    }

    /// At '"': reads the string; unescaped into *out unless out is null.
    bool string(std::string *out)
    {
      i++;  // opening quote
      while ( i < s.size() )
      {
        char c = s[i];
        if ( c == '"' ) { i++; return true; }
        if ( (unsigned char) c < 0x20 ) return fail("control character in string");
        if ( c != '\\' )
        {
          size_t n = utf8Len(s, i);
          if ( n == 0 ) return fail("invalid UTF-8 in string");
          if ( out ) out->append(s.data() + i, n);
          i += n;
          continue;
        }
        if ( ++i >= s.size() ) break;
        char e = s[i++];
        char simple = 0;
        switch ( e )
        {
          case '"': simple = '"'; break;
          case '\\': simple = '\\'; break;
          case '/': simple = '/'; break;
          case 'b': simple = '\b'; break;
          case 'f': simple = '\f'; break;
          case 'n': simple = '\n'; break;
          case 'r': simple = '\r'; break;
          case 't': simple = '\t'; break;
          case 'u':
          {
            unsigned long cp;
            if ( !hex4(cp) ) return false;
            if ( cp >= 0xD800 && cp <= 0xDBFF )
            {
              unsigned long lo;
              if ( i + 2 > s.size() || s[i] != '\\' || s[i + 1] != 'u' ) return fail("lone surrogate");
              i += 2;
              if ( !hex4(lo) ) return false;
              if ( lo < 0xDC00 || lo > 0xDFFF ) return fail("lone surrogate");
              cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
            }
            else if ( cp >= 0xDC00 && cp <= 0xDFFF )
              return fail("lone surrogate");
            if ( out ) appendUtf8(*out, cp);
            continue;
          }
          default: return fail("bad escape");
        }
        if ( out ) *out += simple;
      }
      return fail("unterminated string");
    }

    bool number()
    {
      size_t start = i;
      if ( i < s.size() && s[i] == '-' ) i++;
      if ( i >= s.size() || !std::isdigit((unsigned char) s[i]) ) return fail("bad number");
      if ( s[i] == '0' ) i++;
      else while ( i < s.size() && std::isdigit((unsigned char) s[i]) ) i++;
      if ( i < s.size() && s[i] == '.' )
      {
        i++;
        if ( i >= s.size() || !std::isdigit((unsigned char) s[i]) ) return fail("bad number");
        while ( i < s.size() && std::isdigit((unsigned char) s[i]) ) i++;
      }
      if ( i < s.size() && (s[i] == 'e' || s[i] == 'E') )
      {
        i++;
        if ( i < s.size() && (s[i] == '+' || s[i] == '-') ) i++;
        if ( i >= s.size() || !std::isdigit((unsigned char) s[i]) ) return fail("bad number");
        while ( i < s.size() && std::isdigit((unsigned char) s[i]) ) i++;
      }
      return i > start;
    }

    bool literal(const char *word)
    {
      std::string_view w(word);
      if ( s.substr(i, w.size()) != w ) return fail("unexpected character");
      i += w.size();
      return true;
    }

    /// Skips (and checks) the value at i.
    bool skip(int depth)
    {
      if ( depth > maxDepth ) return fail("nested too deeply");
      ws();
      if ( i >= s.size() ) return fail("unexpected end");
      char c = s[i];
      if ( c == '"' ) return string(nullptr);
      if ( c == '{' || c == '[' )
      {
        char close = c == '{' ? '}' : ']';
        i++;
        ws();
        if ( i < s.size() && s[i] == close ) { i++; return true; }
        for (;;)
        {
          if ( c == '{' )
          {
            ws();
            if ( i >= s.size() || s[i] != '"' ) return fail("expected member name");
            if ( !string(nullptr) ) return false;
            ws();
            if ( i >= s.size() || s[i] != ':' ) return fail("expected ':'");
            i++;
          }
          if ( !skip(depth + 1) ) return false;
          ws();
          if ( i < s.size() && s[i] == ',' ) { i++; continue; }
          if ( i < s.size() && s[i] == close ) { i++; return true; }
          return fail(c == '{' ? "expected ',' or '}'" : "expected ',' or ']'");
        }
      }
      if ( c == 't' ) return literal("true");
      if ( c == 'f' ) return literal("false");
      if ( c == 'n' ) return literal("null");
      return number();
    }

    /// Reads the value at i into out.
    bool value(Value &out)
    {
      ws();
      if ( i >= s.size() ) return fail("unexpected end");
      char c = s[i];
      size_t start = i;
      if ( c == '"' )
      {
        out.kind = Kind::String;
        out.text.clear();
        return string(&out.text);
      }
      if ( !skip(0) ) return false;
      out.text.assign(s.data() + start, i - start);
      out.kind = c == '{' ? Kind::Object : c == '[' ? Kind::Array : c == 't' || c == 'f' ? Kind::Bool
               : c == 'n' ? Kind::Null : Kind::Number;
      return true;
    }

    /// Moves i into the value of member `key` of the object at i.
    /// found = false (and true returned) if the object has no such member.
    bool member(const std::string &key, bool &found)
    {
      found = false;
      i++;  // '{'
      ws();
      if ( i < s.size() && s[i] == '}' ) { i++; return true; }
      for (;;)
      {
        ws();
        if ( i >= s.size() || s[i] != '"' ) return fail("expected member name");
        std::string name;
        if ( !string(&name) ) return false;
        ws();
        if ( i >= s.size() || s[i] != ':' ) return fail("expected ':'");
        i++;
        ws();
        if ( name == key ) { found = true; return true; }
        if ( !skip(1) ) return false;
        ws();
        if ( i < s.size() && s[i] == ',' ) { i++; continue; }
        if ( i < s.size() && s[i] == '}' ) { i++; return true; }
        return fail("expected ',' or '}'");
      }
    }

    /// Moves i into element `index` of the array at i.
    bool element(size_t index, bool &found)
    {
      found = false;
      i++;  // '['
      ws();
      if ( i < s.size() && s[i] == ']' ) { i++; return true; }
      for (size_t k = 0;; k++)
      {
        ws();
        if ( k == index ) { found = true; return true; }
        if ( !skip(1) ) return false;
        ws();
        if ( i < s.size() && s[i] == ',' ) { i++; continue; }
        if ( i < s.size() && s[i] == ']' ) { i++; return true; }
        return fail("expected ',' or ']'");
      }
    }
  };

  // ---------------------------------------------------------------- builder

  struct Node
  {
    enum Type { Unset, Leaf, Obj, Arr } type = Unset;
    std::string raw;
    std::vector<std::pair<std::string, std::unique_ptr<Node>>> members;
    std::vector<std::unique_ptr<Node>> elements;  // nullptr = null

    void write(std::string &out) const
    {
      switch ( type )
      {
        case Leaf: out += raw; break;
        case Obj:
          out += '{';
          for (size_t k = 0; k < members.size(); k++)
          {
            if ( k ) out += ',';
            out += quote(members[k].first);
            out += ':';
            members[k].second->write(out);
          }
          out += '}';
          break;
        case Arr:
          out += '[';
          for (size_t k = 0; k < elements.size(); k++)
          {
            if ( k ) out += ',';
            if ( elements[k] ) elements[k]->write(out);
            else out += "null";
          }
          out += ']';
          break;
        default: out += "null"; break;
      }
    }
  };

  // days since 1970-01-01 of a civil date (H. Hinnant's algorithm)
  long long daysFromCivil(long long y, unsigned m, unsigned d)
  {
    y -= m <= 2;
    long long era = (y >= 0 ? y : y - 399) / 400;
    unsigned yoe = (unsigned) (y - era * 400);
    unsigned doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    unsigned doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + (long long) doe - 719468;
  }

  void civilFromDays(long long z, long long &y, unsigned &m, unsigned &d)
  {
    z += 719468;
    long long era = (z >= 0 ? z : z - 146096) / 146097;
    unsigned doe = (unsigned) (z - era * 146097);
    unsigned yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    y = (long long) yoe + era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    unsigned mp = (5 * doy + 2) / 153;
    d = doy - (153 * mp + 2) / 5 + 1;
    m = mp < 10 ? mp + 3 : mp - 9;
    y += m <= 2;
  }

  bool digits(std::string_view s, size_t &i, size_t n, int &out)
  {
    if ( i + n > s.size() ) return false;
    out = 0;
    for (size_t k = 0; k < n; k++)
    {
      char c = s[i + k];
      if ( c < '0' || c > '9' ) return false;
      out = out * 10 + (c - '0');
    }
    i += n;
    return true;
  }

  bool parseIso(std::string_view s, long long &ms)
  {
    size_t i = 0;
    int y, mo, d, h, mi, sec;
    if ( !digits(s, i, 4, y) || i >= s.size() || s[i++] != '-' || !digits(s, i, 2, mo) ||
         i >= s.size() || s[i++] != '-' || !digits(s, i, 2, d) )
      return false;
    if ( i >= s.size() || (s[i] != 'T' && s[i] != 't' && s[i] != ' ') ) return false;
    i++;
    if ( !digits(s, i, 2, h) || i >= s.size() || s[i++] != ':' || !digits(s, i, 2, mi) ) return false;
    sec = 0;
    if ( i < s.size() && s[i] == ':' )
    {
      i++;
      if ( !digits(s, i, 2, sec) ) return false;
    }
    long long frac = 0;
    if ( i < s.size() && (s[i] == '.' || s[i] == ',') )
    {
      i++;
      int n = 0;
      while ( i < s.size() && s[i] >= '0' && s[i] <= '9' )
      {
        if ( n < 3 ) frac = frac * 10 + (s[i] - '0');
        n++;
        i++;
      }
      if ( n == 0 ) return false;
      for (; n < 3; n++) frac *= 10;
    }
    long long offset = 0;  // minutes east of UTC
    if ( i < s.size() )
    {
      char c = s[i++];
      if ( c == 'Z' || c == 'z' ) {}
      else if ( c == '+' || c == '-' )
      {
        int oh, om = 0;
        if ( !digits(s, i, 2, oh) ) return false;
        if ( i < s.size() && s[i] == ':' ) i++;
        if ( i < s.size() && !digits(s, i, 2, om) ) return false;
        offset = (oh * 60 + om) * (c == '-' ? -1 : 1);
      }
      else return false;
    }
    if ( i != s.size() ) return false;
    if ( mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59 || sec > 60 ) return false;
    long long days = daysFromCivil(y, (unsigned) mo, (unsigned) d);
    ms = (((days * 24 + h) * 60 + mi - offset) * 60 + sec) * 1000 + frac;
    return true;
  }
}

//------------------------------------------------------------------------------

bool parsePath(std::string_view text, Path &out, std::string &err)
{
  out.clear();
  size_t i = 0;
  // JSONPath style root: "$", "$.a", "$[0]" (but "$value" is a member name)
  if ( text == "$" ) return true;
  if ( text.size() > 1 && text[0] == '$' && (text[1] == '.' || text[1] == '[') )
    i = text[1] == '.' ? 2 : 1;
  bool expectName = true;  // at the start or after '.'
  while ( i < text.size() )
  {
    char c = text[i];
    if ( c == '[' )
    {
      i++;
      Segment seg;
      if ( i < text.size() && (text[i] == '"' || text[i] == '\'') )
      {
        char q = text[i++];
        size_t end = text.find(q, i);
        if ( end == std::string_view::npos || end + 1 >= text.size() || text[end + 1] != ']' )
        {
          err = "unterminated [\"...\"] in JSON path";
          return false;
        }
        seg.key = std::string(text.substr(i, end - i));
        i = end + 2;
      }
      else
      {
        size_t end = text.find(']', i);
        if ( end == std::string_view::npos || end == i )
        {
          err = "expected [<index>] in JSON path";
          return false;
        }
        size_t idx = 0;
        auto r = std::from_chars(text.data() + i, text.data() + end, idx);
        if ( r.ec != std::errc() || r.ptr != text.data() + end || idx > 100000 )
        {
          err = "array index in JSON path must be a number up to 100000";
          return false;
        }
        seg.isIndex = true;
        seg.index = idx;
        i = end + 1;
      }
      out.push_back(std::move(seg));
      expectName = false;
      continue;
    }
    if ( c == '.' )
    {
      if ( expectName )
      {
        err = "empty member name in JSON path";
        return false;
      }
      i++;
      expectName = true;
      if ( i == text.size() )
      {
        err = "JSON path ends with '.'";
        return false;
      }
      continue;
    }
    if ( !expectName )
    {
      err = "expected '.' or '[' in JSON path";
      return false;
    }
    size_t end = text.find_first_of(".[]", i);
    if ( end == std::string_view::npos ) end = text.size();
    if ( end < text.size() && text[end] == ']' )
    {
      err = "unexpected ']' in JSON path";
      return false;
    }
    Segment seg;
    seg.key = std::string(text.substr(i, end - i));
    out.push_back(std::move(seg));
    i = end;
    expectName = false;
  }
  return true;
}

std::string toString(const Path &path)
{
  if ( path.empty() ) return "$";
  std::string s;
  for (const auto &seg : path)
  {
    if ( seg.isIndex )
      s += "[" + std::to_string(seg.index) + "]";
    else if ( seg.key.empty() || seg.key.find_first_of(".[]") != std::string::npos )
      s += "[\"" + seg.key + "\"]";
    else
    {
      if ( !s.empty() ) s += '.';
      s += seg.key;
    }
  }
  return s;
}

Found get(std::string_view doc, const Path &path, Value &out, std::string &err)
{
  Parser p;
  p.s = doc;
  for (const auto &seg : path)
  {
    p.ws();
    if ( p.i >= doc.size() )
    {
      p.fail("unexpected end");
      err = p.err;
      return Found::NotJson;
    }
    char c = doc[p.i];
    bool found = false;
    if ( seg.isIndex ? c != '[' : c != '{' )
    {
      // a value of another type: check it is JSON at all
      Value ignored;
      if ( !p.value(ignored) ) { err = p.err; return Found::NotJson; }
      return Found::Missing;
    }
    if ( !(seg.isIndex ? p.element(seg.index, found) : p.member(seg.key, found)) )
    {
      err = p.err;
      return Found::NotJson;
    }
    if ( !found ) return Found::Missing;
  }
  if ( !p.value(out) )
  {
    err = p.err;
    return Found::NotJson;
  }
  if ( path.empty() )
  {
    p.ws();
    if ( p.i != doc.size() )
    {
      err = "unexpected data after the JSON value at offset " + std::to_string(p.i);
      return Found::NotJson;
    }
  }
  return Found::Yes;
}

//------------------------------------------------------------------------------

int Extractor::add(const Path &path)
{
  int n = 0;
  for (const auto &seg : path)
  {
    // (no references into `nodes` across push_back)
    int next;
    if ( seg.isIndex )
    {
      auto it = nodes[n].elements.find(seg.index);
      if ( it != nodes[n].elements.end() ) next = it->second;
      else { next = (int) nodes.size(); nodes[n].elements[seg.index] = next; nodes.emplace_back(); }
    }
    else
    {
      auto it = nodes[n].members.find(seg.key);
      if ( it != nodes[n].members.end() ) next = it->second;
      else { next = (int) nodes.size(); nodes[n].members[seg.key] = next; nodes.emplace_back(); }
    }
    n = next;
  }
  if ( nodes[n].id < 0 )
    nodes[n].id = count++;
  return nodes[n].id;
}

void Extractor::clear()
{
  nodes.assign(1, Node());
  count = 0;
}

void Extractor::run(std::string_view doc, std::vector<Result> &results, std::string &err) const
{
  results.resize((size_t) count);
  for (auto &r : results)
  {
    r.found = Found::Missing;
    r.value.kind = Kind::Null;
    r.value.text.clear();
  }
  err.clear();
  if ( count == 0 )
    return;

  // Walks the document once. Values on a requested path are recorded, their
  // containers entered only where a requested path continues; everything
  // else is skipped (and checked). Like get(), the first of duplicate
  // member names counts.
  struct Walker
  {
    const std::vector<Node> &nodes;
    std::vector<Result> &results;
    Parser p;
    std::vector<char> visited;
    int remaining;
    std::vector<char> done;  // per result: found, or known to be missing
    int openTargets = 0;     // requested containers being read: no early stop inside
    bool stop = false;

    void settle(int id)
    {
      if ( id < 0 || done[(size_t) id] ) return;
      done[(size_t) id] = 1;
      if ( --remaining == 0 && openTargets == 0 )
        stop = true;  // nothing left to look for
    }

    // Requested paths below node ni can no longer be found (missing member,
    // short array, value of another type): Missing, as get() says right there.
    void missingBelow(int ni)
    {
      for (const auto &m : nodes[(size_t) ni].members)  { missingBelow(m.second); settle(nodes[(size_t) m.second].id); }
      for (const auto &e : nodes[(size_t) ni].elements) { missingBelow(e.second); settle(nodes[(size_t) e.second].id); }
    }

    bool walk(int ni, int depth)
    {
      if ( depth > maxDepth ) return p.fail("nested too deeply");
      const Node &n = nodes[ni];
      p.ws();
      if ( p.i >= p.s.size() ) return p.fail("unexpected end");
      char c = p.s[p.i];
      size_t start = p.i;

      if ( c == '"' )
      {
        if ( n.id < 0 )
        {
          if ( !p.string(nullptr) ) return false;
          missingBelow(ni);
          return true;
        }
        Result &r = results[(size_t) n.id];
        r.value.kind = Kind::String;
        if ( !p.string(&r.value.text) ) return false;
        missingBelow(ni);
        found(r, n.id);
        return true;
      }

      bool target = n.id >= 0;
      if ( target ) openTargets++;
      bool ok;
      if ( c == '{' && !n.members.empty() )      ok = members(ni, depth);
      else if ( c == '[' && !n.elements.empty() ) ok = elements(ni, depth);
      else
      {
        ok = p.skip(depth);
        if ( ok ) missingBelow(ni);  // another type than the paths below need
      }
      if ( target ) openTargets--;
      if ( !ok || stop ) return ok;

      if ( target )
      {
        Result &r = results[(size_t) n.id];
        r.value.text.assign(p.s.data() + start, p.i - start);
        r.value.kind = c == '{' ? Kind::Object : c == '[' ? Kind::Array : c == 't' || c == 'f' ? Kind::Bool
                     : c == 'n' ? Kind::Null : Kind::Number;
        found(r, n.id);
      }
      return true;
    }

    void found(Result &r, int id)
    {
      r.found = Found::Yes;
      settle(id);
    }

    bool child(int ni, int depth)
    {
      if ( ni < 0 || visited[(size_t) ni] ) return p.skip(depth);
      visited[(size_t) ni] = 1;
      return walk(ni, depth);
    }

    // the end of an object or array: what was not in it is missing
    void notVisited(int ni)
    {
      const Node &n = nodes[(size_t) ni];
      for (const auto &m : n.members)
        if ( !visited[(size_t) m.second] ) { missingBelow(m.second); settle(nodes[(size_t) m.second].id); }
      for (const auto &e : n.elements)
        if ( !visited[(size_t) e.second] ) { missingBelow(e.second); settle(nodes[(size_t) e.second].id); }
    }

    bool members(int ni, int depth)
    {
      const Node &n = nodes[(size_t) ni];
      p.i++;  // '{'
      p.ws();
      if ( p.i < p.s.size() && p.s[p.i] == '}' ) { p.i++; notVisited(ni); return true; }
      std::string name;
      for (;;)
      {
        p.ws();
        if ( p.i >= p.s.size() || p.s[p.i] != '"' ) return p.fail("expected member name");
        name.clear();
        if ( !p.string(&name) ) return false;
        p.ws();
        if ( p.i >= p.s.size() || p.s[p.i] != ':' ) return p.fail("expected ':'");
        p.i++;
        auto it = n.members.find(name);
        if ( !child(it == n.members.end() ? -1 : it->second, depth + 1) ) return false;
        if ( stop ) return true;
        p.ws();
        if ( p.i < p.s.size() && p.s[p.i] == ',' ) { p.i++; continue; }
        if ( p.i < p.s.size() && p.s[p.i] == '}' ) { p.i++; notVisited(ni); return true; }
        return p.fail("expected ',' or '}'");
      }
    }

    bool elements(int ni, int depth)
    {
      const Node &n = nodes[(size_t) ni];
      p.i++;  // '['
      p.ws();
      if ( p.i < p.s.size() && p.s[p.i] == ']' ) { p.i++; notVisited(ni); return true; }
      for (size_t k = 0;; k++)
      {
        auto it = n.elements.find(k);
        if ( !child(it == n.elements.end() ? -1 : it->second, depth + 1) ) return false;
        if ( stop ) return true;
        p.ws();
        if ( p.i < p.s.size() && p.s[p.i] == ',' ) { p.i++; continue; }
        if ( p.i < p.s.size() && p.s[p.i] == ']' ) { p.i++; notVisited(ni); return true; }
        return p.fail("expected ',' or ']'");
      }
    }
  };

  Walker w{nodes, results, Parser(), std::vector<char>(nodes.size(), 0), count, std::vector<char>((size_t) count, 0)};
  w.p.s = doc;
  w.visited[0] = 1;
  bool ok = w.walk(0, 0);
  if ( !ok )
  {
    // like get(): what was settled before the error stands, the rest is NotJson
    err = w.p.err;
    for (size_t k = 0; k < results.size(); k++)
      if ( !w.done[k] ) results[k].found = Found::NotJson;
    return;
  }
  // the whole payload as the value (empty path): nothing may follow it
  // (the document is the outermost target, so it was read completely)
  int rootId = nodes[0].id;
  if ( rootId >= 0 && results[(size_t) rootId].found == Found::Yes )
  {
    w.p.ws();
    if ( w.p.i != doc.size() )
    {
      err = "unexpected data after the JSON value at offset " + std::to_string(w.p.i);
      results[(size_t) rootId].found = Found::NotJson;
    }
  }
}

bool parseArray(std::string_view array, std::vector<Value> &out, std::string &err)
{
  out.clear();
  Parser p;
  p.s = array;
  p.ws();
  if ( p.i >= array.size() || array[p.i] != '[' )
  {
    err = "not a JSON array";
    return false;
  }
  p.i++;
  p.ws();
  if ( p.i < array.size() && array[p.i] == ']' )
    p.i++;
  else
    for (;;)
    {
      Value v;
      if ( !p.value(v) ) { err = p.err; return false; }
      out.push_back(std::move(v));
      p.ws();
      if ( p.i < array.size() && array[p.i] == ',' ) { p.i++; continue; }
      if ( p.i < array.size() && array[p.i] == ']' ) { p.i++; break; }
      p.fail("expected ',' or ']'");
      err = p.err;
      return false;
    }
  p.ws();
  if ( p.i != array.size() )
  {
    err = "unexpected data after the JSON array at offset " + std::to_string(p.i);
    return false;
  }
  return true;
}

std::string quote(std::string_view text)
{
  static const char hex[] = "0123456789abcdef";
  std::string out;
  out.reserve(text.size() + 2);
  out += '"';
  for (size_t i = 0; i < text.size();)
  {
    unsigned char c = (unsigned char) text[i];
    if ( c >= 0x80 )
    {
      size_t n = utf8Len(text, i);
      if ( n == 0 ) { out += "\xEF\xBF\xBD"; i++; }  // U+FFFD
      else { out.append(text.data() + i, n); i += n; }
      continue;
    }
    switch ( c )
    {
      case '"':  out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\b': out += "\\b"; break;
      case '\f': out += "\\f"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if ( c < 0x20 ) { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
        else out += (char) c;
    }
    i++;
  }
  out += '"';
  return out;
}

bool build(const std::vector<std::pair<const Path *, std::string>> &members, std::string &out, std::string &err)
{
  Node root;
  for (const auto &m : members)
  {
    Node *n = &root;
    for (const auto &seg : *m.first)
    {
      if ( seg.isIndex )
      {
        if ( n->type == Node::Unset ) n->type = Node::Arr;
        if ( n->type != Node::Arr ) { err = "JSON path " + toString(*m.first) + " collides with another one"; return false; }
        if ( n->elements.size() <= seg.index ) n->elements.resize(seg.index + 1);
        if ( !n->elements[seg.index] ) n->elements[seg.index] = std::make_unique<Node>();
        n = n->elements[seg.index].get();
      }
      else
      {
        if ( n->type == Node::Unset ) n->type = Node::Obj;
        if ( n->type != Node::Obj ) { err = "JSON path " + toString(*m.first) + " collides with another one"; return false; }
        Node *next = nullptr;
        for (auto &kv : n->members)
          if ( kv.first == seg.key ) next = kv.second.get();
        if ( !next )
        {
          n->members.emplace_back(seg.key, std::make_unique<Node>());
          next = n->members.back().second.get();
        }
        n = next;
      }
    }
    if ( n->type != Node::Unset ) { err = "JSON path " + toString(*m.first) + " collides with another one"; return false; }
    n->type = Node::Leaf;
    n->raw = m.second;
  }
  out.clear();
  root.write(out);
  return true;
}

bool parseTime(const Value &v, long long &ms)
{
  std::string_view t(v.text);
  bool numeric = v.kind == Kind::Number;
  if ( v.kind == Kind::String )
  {
    numeric = !t.empty();
    for (char c : t)
      if ( !(std::isdigit((unsigned char) c) || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E') ) numeric = false;
    if ( !numeric ) return parseIso(t, ms);
  }
  if ( !numeric ) return false;
  double d = 0;
  if ( !t.empty() && t.front() == '+' ) t.remove_prefix(1);
  auto r = std::from_chars(t.data(), t.data() + t.size(), d);
  if ( r.ec != std::errc() || r.ptr != t.data() + t.size() || !std::isfinite(d) ) return false;
  ms = (long long) std::llround(std::fabs(d) >= 1e11 ? d : d * 1000.0);
  return true;
}

std::string isoTime(long long ms)
{
  long long days = ms >= 0 ? ms / 86400000 : -((-ms + 86399999) / 86400000);
  long long rest = ms - days * 86400000;
  long long y;
  unsigned m, d;
  civilFromDays(days, y, m, d);
  char buf[40];
  snprintf(buf, sizeof(buf), "%04lld-%02u-%02uT%02lld:%02lld:%02lld.%03lldZ", y, m, d,
           rest / 3600000, (rest / 60000) % 60, (rest / 1000) % 60, rest % 1000);
  return buf;
}
}
