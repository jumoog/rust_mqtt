// rsmqtt_wincc.hpp - header-only C++17 wrapper of the rsmqtt-wincc C ABI
// (rsmqtt_wincc.h): address syntax, JSON paths, Sparkplug B host
// application, redundancy names and connection settings for the WinCC OA driver.
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
#include "rsmqtt.hpp"

#include <chrono>
#include <cstdint>
#include <functional>
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

  //---------------------------------------------------- connection settings

  /// The Config.* elements of a _RsMqttBroker DP.
  struct BrokerConfig
  {
    bool        active = false;  // not part of the settings: the driver decides
    std::string serverURI;
    std::vector<std::string> serverURIs;
    std::string clientId;
    int         mqttVersion = 0;
    int         keepAlive = 0;
    bool        cleanSession = true;
    std::string username;
    std::string password;
    std::string tlsTrustStore;
    std::string tlsKeyStore;
    std::string tlsPrivateKey;
    bool        tlsVerify = false;
    std::string willTopic;
    std::string willPayload;
    int         willQoS = 0;
    bool        willRetain = false;
    int         subscribeQoS = 1;
    int         publishQoS = 1;
    bool        publishRetain = false;
    std::vector<std::string> options;  // "name=value", any rsmqtt option
  };

  /// Settings of one broker connection (crates/rsmqtt-wincc/src/settings.rs):
  /// the rsmqtt options, default QoS / retain of the addresses, reconnect
  /// interval and the Sparkplug host ids.
  class Settings
  {
    public:
      /// Decrypts a stored password (plain values unchanged); false and err on failure.
      using Reveal = std::function<bool(const std::string &stored, std::string &plain, std::string &err)>;

      /// The driver's defaults for a connection of this name.
      Settings(const std::string &name, int manNum) : s(rsmqtt_wincc_settings_new(name.c_str(), manNum))
      {
        if ( !s )
          throw rsmqtt::Error(RSMQTT_FAILURE, lastError());
      }
      ~Settings() { rsmqtt_wincc_settings_free(s); }
      Settings(const Settings &) = delete;
      Settings &operator=(const Settings &) = delete;

      /// An rsmqtt option (config file).
      bool setOption(const std::string &name, std::string_view value, std::string &err)
      {
        return ok(rsmqtt_wincc_settings_set_option(s, name.c_str(), value.data(), value.size()), err);
      }

      /// sparkplug.hostId (config file).
      bool setSpbHostId(const std::string &id, std::string &err)
      {
        return ok(rsmqtt_wincc_settings_set_spb_host_id(s, id.c_str()), err);
      }

      /// A "redu.<option>" entry (option without "redu."): sets it on the
      /// second redundant host, only checks it on host 1.
      bool applyReduOption(const std::string &option, std::string_view value, bool host2, std::string &err)
      {
        return ok(rsmqtt_wincc_settings_apply_redu_option(s, option.c_str(), value.data(), value.size(), host2), err);
      }

      /// Builds the settings from a broker DP; on failure they stay as they were.
      bool applyBrokerConfig(const BrokerConfig &cfg, bool host2, const Reveal &reveal, std::string &err)
      {
        auto str = [](const std::string &v) { return rsmqtt_wincc_str{v.data(), v.size()}; };
        std::vector<rsmqtt_wincc_str> uris, options;
        for (const auto &u : cfg.serverURIs) uris.push_back(str(u));
        for (const auto &o : cfg.options) options.push_back(str(o));
        rsmqtt_wincc_broker_config c{};
        c.server_uri = str(cfg.serverURI);
        c.server_uris = uris.data();
        c.server_uris_len = uris.size();
        c.client_id = str(cfg.clientId);
        c.mqtt_version = cfg.mqttVersion;
        c.keep_alive = cfg.keepAlive;
        c.clean_session = cfg.cleanSession;
        c.username = str(cfg.username);
        c.password = str(cfg.password);
        c.tls_trust_store = str(cfg.tlsTrustStore);
        c.tls_key_store = str(cfg.tlsKeyStore);
        c.tls_private_key = str(cfg.tlsPrivateKey);
        c.tls_verify = cfg.tlsVerify;
        c.will_topic = str(cfg.willTopic);
        c.will_payload = str(cfg.willPayload);
        c.will_qos = cfg.willQoS;
        c.will_retain = cfg.willRetain;
        c.subscribe_qos = cfg.subscribeQoS;
        c.publish_qos = cfg.publishQoS;
        c.publish_retain = cfg.publishRetain;
        c.options = options.data();
        c.options_len = options.size();
        return ok(rsmqtt_wincc_settings_apply_broker_config(s, &c, host2, reveal ? &callReveal : nullptr,
                                                            const_cast<Reveal *>(&reveal)), err);
      }

      /// The Sparkplug host id this driver publishes STATE for ("" for none).
      std::string spbStateHost(bool host2) const
      {
        rsmqtt_wincc_buf b{};
        if ( rsmqtt_wincc_settings_spb_state_host(s, host2, &b) != 0 )
          return std::string();
        return take(b);
      }

      /// The options to create the client with (STATE will, client id of host 2,
      /// persistence folder below dataDir); warnings for the log.
      rsmqtt::Options clientOptions(bool host2, const std::string &dataDir, int64_t nowMs, std::vector<std::string> &warnings)
      {
        rsmqtt_wincc_buf w{};
        rsmqtt_options *o = rsmqtt_wincc_settings_client_options(s, host2, dataDir.c_str(), nowMs, &w);
        if ( !o )
          throw rsmqtt::Error(RSMQTT_FAILURE, lastError());
        std::string text = take(w);
        for (size_t b = 0; b < text.size();)
        {
          size_t e = text.find('\n', b);
          if ( e == std::string::npos ) e = text.size();
          warnings.push_back(text.substr(b, e - b));
          b = e + 1;
        }
        return rsmqtt::Options::adopt(o);
      }

      int     subscribeQoS() const { return (int) get(RSMQTT_WINCC_SETTINGS_SUBSCRIBE_QOS); }
      int     publishQoS() const { return (int) get(RSMQTT_WINCC_SETTINGS_PUBLISH_QOS); }
      bool    publishRetain() const { return get(RSMQTT_WINCC_SETTINGS_PUBLISH_RETAIN) != 0; }
      std::chrono::milliseconds reconnectInterval() const
      {
        return std::chrono::milliseconds(get(RSMQTT_WINCC_SETTINGS_RECONNECT_INTERVAL_MS));
      }
      bool    autoReconnect() const { return get(RSMQTT_WINCC_SETTINGS_AUTO_RECONNECT) != 0; }
      /// Timestamp of the STATE messages of the current client (set by clientOptions).
      int64_t stateTs() const { return get(RSMQTT_WINCC_SETTINGS_STATE_TS); }

      void setSubscribeQoS(int q) { rsmqtt_wincc_settings_set(s, RSMQTT_WINCC_SETTINGS_SUBSCRIBE_QOS, q); }
      void setPublishQoS(int q) { rsmqtt_wincc_settings_set(s, RSMQTT_WINCC_SETTINGS_PUBLISH_QOS, q); }
      void setPublishRetain(bool r) { rsmqtt_wincc_settings_set(s, RSMQTT_WINCC_SETTINGS_PUBLISH_RETAIN, r); }
      void setReconnectInterval(std::chrono::milliseconds i)
      {
        rsmqtt_wincc_settings_set(s, RSMQTT_WINCC_SETTINGS_RECONNECT_INTERVAL_MS, (int64_t) i.count());
      }

      /// Options whose value may be encrypted (password).
      static bool isSecretOption(const std::string &name) { return rsmqtt_wincc_is_secret_option(name.c_str()) != 0; }

    private:
      int64_t get(int field) const { return rsmqtt_wincc_settings_get(s, field); }

      static bool ok(int rc, std::string &err)
      {
        if ( rc == 0 )
          return true;
        err = lastError();
        return false;
      }

      static int callReveal(void *ctx, const char *stored, size_t len, rsmqtt_wincc_buf *out) noexcept
      {
        std::string plain, err;
        bool done = false;
        try
        {
          done = (*static_cast<const Reveal *>(ctx))(std::string(stored ? stored : "", len), plain, err);
        }
        catch (const std::exception &e)
        {
          err = e.what();
        }
        catch (...)
        {
          err = "decryption failed";
        }
        const std::string &result = done ? plain : err;
        *out = rsmqtt_wincc_buf_new(result.data(), result.size());
        return done ? 0 : -1;
      }

      rsmqtt_wincc_settings *s;
  };

  //--------------------------------------------------------- message router

  /// The addresses of one broker connection and how messages reach them
  /// (crates/rsmqtt-wincc/src/router.rs).
  class Router
  {
    public:
      enum class Added { Nothing = 0, State = 1, Subscribe = 2, Error = -1 };

      struct Delivery
      {
        const char      *address = "";
        bool             hasValue = false;  // false: the payload of the message as it is
        std::string_view value;
        bool             hasTime = false;   // false: now
        int64_t          timeMs = 0;
        bool             periphTime = false;  // the value's own time (ts= / Sparkplug)
        int              validity = RSMQTT_WINCC_VALIDITY_KEEP;
      };

      struct Publish
      {
        const char      *topic = "";
        std::string_view payload;
        int              qos = 0;
        bool             retain = false;
      };

      /// Results of a call; the views stay valid until the next call that
      /// changes the router.
      struct Outcome
      {
        std::vector<Delivery> deliveries;
        std::vector<Publish> publish;
        std::vector<std::pair<const char *, int>> subscribe;   // (filter, QoS)
        std::vector<std::pair<bool, const char *>> warnings;   // (from Sparkplug, text)
      };

      Router() : r(rsmqtt_wincc_router_new()) {}
      ~Router() { rsmqtt_wincc_router_free(r); }
      Router(const Router &) = delete;
      Router &operator=(const Router &) = delete;

      /// Registers an address of a DPE; Subscribe: subscribe `filter`.
      Added add(const std::string &fullAddress, bool isDyn, bool input, int defaultQoS, std::string &filter,
                std::string &err)
      {
        rsmqtt_wincc_buf b{};
        int rc = rsmqtt_wincc_router_add(r, fullAddress.c_str(), isDyn, input, defaultQoS, &b);
        if ( rc < 0 )
        {
          err = lastError();
          return Added::Error;
        }
        if ( rc == RSMQTT_WINCC_ADDED_SUBSCRIBE )
          filter = take(b);
        return (Added) rc;
      }

      /// Unregisters an address; true: unsubscribe `filter`.
      bool remove(const std::string &fullAddress, bool input, std::string &filter)
      {
        rsmqtt_wincc_buf b{};
        if ( rsmqtt_wincc_router_remove(r, fullAddress.c_str(), input, &b) != 1 )
          return false;
        filter = take(b);
        return true;
      }

      bool filter(const std::string &f, int &qos, int &flags) const
      {
        return rsmqtt_wincc_router_filter(r, f.c_str(), &qos, &flags) == 1;
      }

      std::vector<std::string> filters() const
      {
        std::vector<std::string> out;
        rsmqtt_wincc_buf b{};
        if ( rsmqtt_wincc_router_filters(r, &b) != 0 )
          return out;
        std::string all = take(b);
        for (size_t p = 0; p < all.size();)
        {
          size_t e = all.find('\0', p);
          if ( e == std::string::npos ) e = all.size();
          out.push_back(all.substr(p, e - p));
          p = e + 1;
        }
        return out;
      }

      bool hasAddresses() const { return rsmqtt_wincc_router_has_addresses(r) != 0; }

      /// One message; stateHost / stateTs: the Sparkplug STATE this client publishes ("" for none).
      const Outcome &message(const std::string &topic, std::string_view payload, int64_t nowMs,
                             const std::string &stateHost, int64_t stateTs)
      {
        rsmqtt_wincc_router_message(r, topic.c_str(), payload.data(), payload.size(), nowMs, stateHost.c_str(), stateTs);
        return collect();
      }

      /// The values of the $state addresses.
      const Outcome &state(bool connected)
      {
        rsmqtt_wincc_router_state(r, connected);
        return collect();
      }

      /// The connection is established.
      const Outcome &connected(const std::string &stateHost, int64_t stateTs)
      {
        rsmqtt_wincc_router_connected(r, stateHost.c_str(), stateTs);
        return collect();
      }

      /// The connection is gone: Sparkplug edge nodes offline, their metrics stale.
      const Outcome &connectionLost()
      {
        rsmqtt_wincc_router_connection_lost(r);
        return collect();
      }

      struct Write
      {
        std::string topic;
        std::string payload;
        int         qos = 0;
        bool        retain = false;
      };

      /// The message that writes `value` (payload text of data type valueType)
      /// to an output address; false and `err` (for the log) if it cannot be written.
      bool write(const std::string &fullAddress, std::string_view value, int valueType, int64_t timeMs, int64_t nowMs,
                 int publishQoS, bool publishRetain, Write &out, std::string &err) const
      {
        rsmqtt_wincc_buf t{}, p{};
        int qos = 0, retain = 0;
        if ( rsmqtt_wincc_router_write(r, fullAddress.c_str(), value.data(), value.size(), valueType, timeMs, nowMs,
                                       publishQoS, publishRetain, &t, &p, &qos, &retain) != 0 )
        {
          err = lastError();
          return false;
        }
        out.topic = take(t);
        out.payload = take(p);
        out.qos = qos;
        out.retain = retain != 0;
        return true;
      }

    private:
      const Outcome &collect()
      {
        out.deliveries.clear();
        out.publish.clear();
        out.subscribe.clear();
        out.warnings.clear();
        for (size_t i = 0, n = rsmqtt_wincc_router_delivery_count(r); i < n; i++)
        {
          rsmqtt_wincc_delivery d{};
          if ( rsmqtt_wincc_router_delivery_get(r, i, &d) != 0 ) continue;
          Delivery x;
          x.address = d.address;
          x.hasValue = d.has_value != 0;
          x.value = std::string_view(d.value ? d.value : "", d.value_len);
          x.hasTime = d.has_time != 0;
          x.timeMs = d.time_ms;
          x.periphTime = d.periph_time != 0;
          x.validity = d.validity;
          out.deliveries.push_back(x);
        }
        for (size_t i = 0, n = rsmqtt_wincc_router_publish_count(r); i < n; i++)
        {
          rsmqtt_wincc_publish p{};
          if ( rsmqtt_wincc_router_publish_get(r, i, &p) != 0 ) continue;
          out.publish.push_back({p.topic, std::string_view(p.payload ? p.payload : "", p.payload_len), p.qos, p.retain != 0});
        }
        for (size_t i = 0, n = rsmqtt_wincc_router_subscribe_count(r); i < n; i++)
        {
          const char *f = nullptr;
          int q = 0;
          if ( rsmqtt_wincc_router_subscribe_get(r, i, &f, &q) == 0 ) out.subscribe.emplace_back(f, q);
        }
        for (size_t i = 0, n = rsmqtt_wincc_router_warning_count(r); i < n; i++)
        {
          int spb = 0;
          const char *w = rsmqtt_wincc_router_warning_get(r, i, &spb);
          out.warnings.emplace_back(spb != 0, w);
        }
        return out;
      }

      rsmqtt_wincc_router *r;
      Outcome out;
  };

  //------------------------------------------------------------------ values

  namespace values
  {
    /// A number or boolean of data type `type` (1002 int32 .. 1007 bool) from
    /// payload text (crates/rsmqtt-wincc/src/values.rs); false and `why` if
    /// it cannot be converted.
    inline bool parseNumber(int type, std::string_view text, bool jsonString, rsmqtt_wincc_number &out, std::string &why)
    {
      if ( rsmqtt_wincc_parse_number(type, text.data(), text.size(), jsonString, &out) == 0 )
        return true;
      why = lastError();
      return false;
    }
  }
}

#endif
