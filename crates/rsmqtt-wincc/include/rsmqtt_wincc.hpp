// rsmqtt_wincc.hpp - header-only C++17 wrapper of the rsmqtt-wincc C ABI
// (rsmqtt_wincc.h): address syntax, JSON paths, Sparkplug B host
// application and redundancy names for the WinCC OA driver.
//
//   rsmqtt::wincc::Address a;
//   std::string err;
//   if ( rsmqtt::wincc::Address::parse("plant:s/1;json=data.temp", a, err) ) ...
//
//   rsmqtt::wincc::json::Extractor x;
//   int id = x.add("data.temp");
//   x.run(payload);
//   std::string_view v; rsmqtt::wincc::json::Kind k;
//   if ( x.result(id, k, v) == rsmqtt::wincc::json::Found::Yes ) ...
#ifndef RSMQTT_WINCC_HPP
#define RSMQTT_WINCC_HPP

#include "rsmqtt_wincc.h"

#include <cstdint>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace rsmqtt::wincc
{
  /// Takes an owned buffer into a std::string.
  inline std::string take(rsmqtt_wincc_buf b)
  {
    std::string s(b.data ? b.data : "", b.len);
    rsmqtt_wincc_buf_free(b);
    return s;
  }

  inline std::string lastError()
  {
    const char *e = rsmqtt_wincc_last_error();
    return e ? e : "";
  }

  //-------------------------------------------------------------- addresses

  /// <connection>:<topic>[;options] (see crates/rsmqtt-wincc/src/address.rs)
  struct Address
  {
    std::string connection;
    std::string topic;        // Sparkplug: the edge node's subscription filter
    int  qos = -1;            // -1: connection default
    int  retain = -1;         // -1: connection default
    int  subFlags = 0;        // RSMQTT_SUB_* flags
    bool isState = false;     // "$state" pseudo topic
    bool json = false;        // json[=path]
    std::string jsonPath;     // as written
    bool hasTs = false;       // ts=path
    std::string tsPath;
    bool sparkplug = false;   // $spb/<group>/<edge node>[/<device>]
    std::string spbGroup, spbNode, spbDevice, spbMetric;
    bool spbOnline = false;   // ;online instead of ;metric=

    static bool parse(std::string_view text, Address &out, std::string &err)
    {
      rsmqtt_wincc_address *a = rsmqtt_wincc_address_parse(text.data(), text.size());
      if ( !a )
      {
        err = lastError();
        return false;
      }
      auto s = [a](int f) { return std::string(rsmqtt_wincc_address_text(a, f)); };
      auto i = [a](int f) { return rsmqtt_wincc_address_int(a, f); };
      out.connection = s(RSMQTT_WINCC_ADDR_CONNECTION);
      out.topic      = s(RSMQTT_WINCC_ADDR_TOPIC);
      out.jsonPath   = s(RSMQTT_WINCC_ADDR_JSON_PATH);
      out.tsPath     = s(RSMQTT_WINCC_ADDR_TS_PATH);
      out.spbGroup   = s(RSMQTT_WINCC_ADDR_SPB_GROUP);
      out.spbNode    = s(RSMQTT_WINCC_ADDR_SPB_NODE);
      out.spbDevice  = s(RSMQTT_WINCC_ADDR_SPB_DEVICE);
      out.spbMetric  = s(RSMQTT_WINCC_ADDR_SPB_METRIC);
      out.qos        = i(RSMQTT_WINCC_ADDR_QOS);
      out.retain     = i(RSMQTT_WINCC_ADDR_RETAIN);
      out.subFlags   = i(RSMQTT_WINCC_ADDR_SUB_FLAGS);
      out.isState    = i(RSMQTT_WINCC_ADDR_IS_STATE) != 0;
      out.json       = i(RSMQTT_WINCC_ADDR_JSON) != 0;
      out.hasTs      = i(RSMQTT_WINCC_ADDR_HAS_TS) != 0;
      out.sparkplug  = i(RSMQTT_WINCC_ADDR_SPARKPLUG) != 0;
      out.spbOnline  = i(RSMQTT_WINCC_ADDR_SPB_ONLINE) != 0;
      rsmqtt_wincc_address_free(a);
      return true;
    }
  };

  inline bool topicMatches(std::string_view filter, std::string_view topic)
  {
    return rsmqtt_wincc_topic_matches(std::string(filter).c_str(), std::string(topic).c_str()) != 0;
  }

  inline bool isWildcardFilter(std::string_view filter)
  {
    return rsmqtt_wincc_is_wildcard_filter(std::string(filter).c_str()) != 0;
  }

  //------------------------------------------------------------------- JSON

  namespace json
  {
    enum class Kind { String = 0, Number = 1, Bool = 2, Null = 3, Object = 4, Array = 5 };
    enum class Found { Yes = 0, Missing = 1, NotJson = 2 };

    struct Value
    {
      Kind kind = Kind::Null;
      std::string text;  // unescaped for strings, the JSON text otherwise
    };

    inline std::string quote(std::string_view text)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_json_quote(text.data(), text.size(), &b);
      return take(b);
    }

    /// The canonical form of a path for messages ("data.temp", "$").
    inline std::string pathString(std::string_view path)
    {
      rsmqtt_wincc_buf b{};
      if ( rsmqtt_wincc_json_path_string(path.data(), path.size(), &b) != 0 )
        return std::string(path);
      return take(b);
    }

    /// Nests JSON values (JSON text) along their paths into one document.
    inline bool build(const std::vector<std::pair<std::string_view, std::string_view>> &members, std::string &out,
                      std::string &err)
    {
      std::vector<const char *> paths, values;
      std::vector<size_t> pathLens, valueLens;
      for (const auto &m : members)
      {
        paths.push_back(m.first.data());
        pathLens.push_back(m.first.size());
        values.push_back(m.second.data());
        valueLens.push_back(m.second.size());
      }
      rsmqtt_wincc_buf b{};
      if ( rsmqtt_wincc_json_build(members.size(), paths.data(), pathLens.data(), values.data(), valueLens.data(), &b) != 0 )
      {
        err = lastError();
        return false;
      }
      out = take(b);
      return true;
    }

    inline bool parseTime(Kind kind, std::string_view text, long long &ms)
    {
      int64_t v = 0;
      if ( rsmqtt_wincc_json_parse_time((int) kind, text.data(), text.size(), &v) != 1 )
        return false;
      ms = (long long) v;
      return true;
    }

    inline std::string isoTime(long long ms)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_json_iso_time((int64_t) ms, &b);
      return take(b);
    }

    /// Several paths, one pass over a document per run().
    class Extractor
    {
      public:
        Extractor() : x(rsmqtt_wincc_extractor_new()) {}
        ~Extractor() { rsmqtt_wincc_extractor_free(x); }
        Extractor(const Extractor &) = delete;
        Extractor &operator=(const Extractor &) = delete;
        Extractor(Extractor &&o) noexcept : x(std::exchange(o.x, nullptr)) {}
        Extractor &operator=(Extractor &&o) noexcept { std::swap(x, o.x); return *this; }

        /// The id of a path, -1 if it is not valid.
        int  add(std::string_view path) { return rsmqtt_wincc_extractor_add(x, path.data(), path.size()); }
        void clear() { rsmqtt_wincc_extractor_clear(x); }
        int  size() const { return rsmqtt_wincc_extractor_size(x); }
        bool empty() const { return size() == 0; }
        void run(std::string_view doc) { rsmqtt_wincc_extractor_run(x, doc.data(), doc.size()); }

        /// The result of path `id` of the last run; `text` stays valid until the next run.
        Found result(int id, Kind &kind, std::string_view &text) const
        {
          int k = (int) Kind::Null;
          const char *t = nullptr;
          size_t n = 0;
          int f = rsmqtt_wincc_extractor_result(x, id, &k, &t, &n);
          kind = (Kind) k;
          text = std::string_view(t ? t : "", n);
          return f < 0 ? Found::Missing : (Found) f;
        }

        /// What was wrong with the document of the last run.
        std::string error() const { return rsmqtt_wincc_extractor_error(x); }

      private:
        rsmqtt_wincc_extractor *x;
    };

    /// One path in one document.
    inline Found get(std::string_view doc, std::string_view path, Value &out, std::string &err)
    {
      Extractor x;
      int id = x.add(path);
      if ( id < 0 )
      {
        err = lastError();
        return Found::NotJson;
      }
      x.run(doc);
      std::string_view t;
      Found f = x.result(id, out.kind, t);
      out.text.assign(t.data(), t.size());
      if ( f == Found::NotJson ) err = x.error();
      return f;
    }

    /// The items of a payload for a dyn DPE: (text, is_string) each.
    inline bool dynItems(std::string_view payload, std::vector<std::pair<std::string, bool>> &items, std::string &err)
    {
      items.clear();
      rsmqtt_wincc_items *it = rsmqtt_wincc_dyn_items(payload.data(), payload.size());
      if ( !it )
      {
        err = lastError();
        return false;
      }
      size_t n = rsmqtt_wincc_items_count(it);
      for (size_t i = 0; i < n; i++)
      {
        const char *t = nullptr;
        size_t len = 0;
        int isString = rsmqtt_wincc_items_get(it, i, &t, &len);
        items.emplace_back(std::string(t ? t : "", len), isString == 1);
      }
      rsmqtt_wincc_items_free(it);
      return true;
    }
  }

  //------------------------------------------------------------- redundancy

  namespace redu
  {
    inline std::string forHost(std::string_view base, bool host2)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_redu_for_host(std::string(base).c_str(), host2, &b);
      return take(b);
    }

    inline bool endsWithSuffix(std::string_view name)
    {
      return rsmqtt_wincc_redu_ends_with_suffix(std::string(name).c_str()) != 0;
    }

    /// "redu.<option>": TRUE and option = "<option>".
    inline bool splitReduOption(std::string_view name, std::string &option)
    {
      rsmqtt_wincc_buf b{};
      if ( rsmqtt_wincc_redu_split_option(std::string(name).c_str(), &b) != 1 )
        return false;
      option = take(b);
      return true;
    }

    inline std::string clientIdForHost(std::string_view clientId, bool host2, bool reduClientIdGiven)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_redu_client_id(std::string(clientId).c_str(), host2, reduClientIdGiven, &b);
      return take(b);
    }
  }

  //------------------------------------------------------- Sparkplug B host

  namespace sparkplug
  {
    struct Update
    {
      enum class Kind { Value = 0, Online = 1, Stale = 2 };
      Kind        kind = Kind::Value;
      std::string group, node, device;  // device empty: the edge node itself
      std::string metric;               // empty for Online
      std::string text;                 // Value: the value
      bool        isString = false;
      bool        online = false;
      bool        hasTime = false;
      int64_t     timeMs = 0;
    };

    struct Message
    {
      std::string topic;
      std::string payload;
    };

    class Host
    {
      public:
        Host() : h(rsmqtt_wincc_spb_host_new()) {}
        ~Host() { rsmqtt_wincc_spb_host_free(h); }
        Host(const Host &) = delete;
        Host &operator=(const Host &) = delete;

        void setRebirthInterval(int64_t ms) { rsmqtt_wincc_spb_set_rebirth_interval(h, ms); }

        void onMessage(std::string_view topic, std::string_view payload, int64_t nowMs, std::vector<Update> &updates,
                       std::vector<Message> &publish, std::vector<std::string> &warnings)
        {
          rsmqtt_wincc_spb_on_message(h, std::string(topic).c_str(), payload.data(), payload.size(), nowMs);
          collect(updates);
          for (size_t i = 0; i < rsmqtt_wincc_spb_publish_count(h); i++)
          {
            const char *t = nullptr, *p = nullptr;
            size_t n = 0;
            if ( rsmqtt_wincc_spb_publish_get(h, i, &t, &p, &n) == 0 )
              publish.push_back({t, std::string(p ? p : "", n)});
          }
          for (size_t i = 0; i < rsmqtt_wincc_spb_warning_count(h); i++)
            warnings.emplace_back(rsmqtt_wincc_spb_warning_get(h, i));
        }

        void onConnectionLost(std::vector<Update> &updates)
        {
          rsmqtt_wincc_spb_connection_lost(h);
          collect(updates);
        }

        /// NCMD (device empty) or DCMD that writes `text` to a metric.
        bool command(const std::string &group, const std::string &node, const std::string &device, const std::string &metric,
                     std::string_view text, int64_t nowMs, Message &out, std::string &err) const
        {
          rsmqtt_wincc_buf t{}, p{};
          if ( rsmqtt_wincc_spb_command(h, group.c_str(), node.c_str(), device.c_str(), metric.c_str(), text.data(),
                                        text.size(), nowMs, &t, &p) != 0 )
          {
            err = lastError();
            return false;
          }
          out.topic = take(t);
          out.payload = take(p);
          return true;
        }

        bool isOnline(const std::string &group, const std::string &node, const std::string &device) const
        {
          return rsmqtt_wincc_spb_is_online(h, group.c_str(), node.c_str(), device.c_str()) != 0;
        }

        uint32_t dataType(const std::string &group, const std::string &node, const std::string &device,
                          const std::string &metric) const
        {
          return rsmqtt_wincc_spb_data_type(h, group.c_str(), node.c_str(), device.c_str(), metric.c_str());
        }

      private:
        void collect(std::vector<Update> &updates)
        {
          for (size_t i = 0; i < rsmqtt_wincc_spb_update_count(h); i++)
          {
            rsmqtt_wincc_spb_update u{};
            if ( rsmqtt_wincc_spb_update_get(h, i, &u) != 0 ) continue;
            Update x;
            x.kind = (Update::Kind) u.kind;
            x.group = u.group;
            x.node = u.node;
            x.device = u.device;
            x.metric = u.metric;
            x.text.assign(u.text ? u.text : "", u.text_len);
            x.isString = u.is_string != 0;
            x.online = u.online != 0;
            x.hasTime = u.has_time != 0;
            x.timeMs = u.time_ms;
            updates.push_back(std::move(x));
          }
        }

        rsmqtt_wincc_spb_host *h;
    };

    inline std::string nodeFilter(const std::string &group, const std::string &node)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_spb_node_filter(group.c_str(), node.c_str(), &b);
      return take(b);
    }

    inline std::string stateTopic(const std::string &hostId)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_spb_state_topic(hostId.c_str(), &b);
      return take(b);
    }

    inline std::string statePayload(bool online, int64_t timestampMs)
    {
      rsmqtt_wincc_buf b{};
      rsmqtt_wincc_spb_state_payload(online, timestampMs, &b);
      return take(b);
    }

    inline bool validId(std::string_view id)
    {
      return rsmqtt_wincc_spb_valid_id(std::string(id).c_str()) != 0;
    }
  }
}

#endif
