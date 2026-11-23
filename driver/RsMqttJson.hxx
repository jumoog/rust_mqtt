// RsMqttJson.hxx - the JSON support of rsmqtt addresses (json=<path>, ts=<path>).
//
// Path syntax:   (empty)          the whole payload is the value
//                temp             member "temp" of the top-level object
//                data.temp        nested member
//                values[0]        array element
//                ["a.b"]          member whose name contains . [ ] (also 'a.b')
//                $ / $.data.temp  a leading "$" (JSONPath style) is ignored
//
// Input:  get() finds the value; strings are unescaped, numbers / true /
//         false are their JSON text, objects and arrays their JSON text.
// Output: build() nests one or more values into objects / arrays along
//         their paths, e.g. {"data":{"temp":21.5,"ts":"..."}}.
//
// This file has no WinCC OA dependencies.
#ifndef RSMQTT_JSON_HXX
#define RSMQTT_JSON_HXX

#include <cstddef>
#include <map>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace RsMqttJson
{
  struct Segment
  {
    bool        isIndex = false;
    std::string key;        // member name (isIndex == false)
    size_t      index = 0;  // array index (isIndex == true)
  };
  using Path = std::vector<Segment>;

  /// Parses a path; on failure returns false and sets err.
  bool parsePath(std::string_view text, Path &out, std::string &err);

  /// The path in a canonical form, for messages ("data.temp", "values[0]", "$").
  std::string toString(const Path &path);

  enum class Kind { String, Number, Bool, Null, Object, Array };

  struct Value
  {
    Kind        kind = Kind::Null;
    std::string text;  // unescaped for strings, the JSON text otherwise
  };

  enum class Found { Yes, Missing, NotJson };

  /// Looks up `path` in the JSON document `doc`. Missing: the document is
  /// JSON but has nothing at that path. NotJson: err says what is wrong.
  Found get(std::string_view doc, const Path &path, Value &out, std::string &err);

  /// Looks up several paths in one pass over a document, with the results
  /// get() would give for each path. Paths are added once (e.g. for all
  /// addresses on a topic) and the extractor runs per message.
  class Extractor
  {
    public:
      struct Result
      {
        Found found = Found::Missing;
        Value value;
      };

      /// Adds a path and returns its id (an index into run()'s results);
      /// the same path twice gives the same id.
      int add(const Path &path);
      void clear();
      bool empty() const { return count == 0; }
      int size() const { return count; }

      /// results[id] for every added path; err says what is wrong when a
      /// result is NotJson. `results` is reused to avoid allocations.
      void run(std::string_view doc, std::vector<Result> &results, std::string &err) const;

    private:
      struct Node
      {
        int id = -1;                         // >= 0: a requested path ends here
        std::map<std::string, int> members;  // member name -> node
        std::map<size_t, int> elements;      // array index -> node
      };
      std::vector<Node> nodes = std::vector<Node>(1);  // nodes[0]: the document
      int count = 0;
  };

  /// The elements of the JSON array `array` (e.g. Value::text of an Array).
  bool parseArray(std::string_view array, std::vector<Value> &out, std::string &err);

  /// A JSON string literal; invalid UTF-8 becomes U+FFFD.
  std::string quote(std::string_view text);

  /// Nests JSON values (already JSON text) along their paths into one
  /// document. Fails if the paths collide (one is the parent of another,
  /// or a level is used as object and as array).
  bool build(const std::vector<std::pair<const Path *, std::string>> &members, std::string &out, std::string &err);

  /// Timestamp of a value: a number or digit string is seconds since 1970
  /// (milliseconds when it is 1e11 or more), a string otherwise ISO 8601
  /// ("2026-11-23T10:00:00.123Z", "+01:00" offsets; no offset = UTC).
  bool parseTime(const Value &v, long long &msSinceEpoch);

  /// ISO 8601 in UTC with milliseconds, e.g. "2026-11-23T10:00:00.123Z".
  std::string isoTime(long long msSinceEpoch);
}

#endif
