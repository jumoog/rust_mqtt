/*
 * rsmqtt_wincc.h - C ABI of rsmqtt-wincc: the WinCC OA driver logic of rsmqtt
 * that does not need the WinCC OA API (address syntax, JSON paths, Sparkplug B
 * host application, redundancy names, connection settings). Implemented in Rust
 * (crates/rsmqtt-wincc); the static library rsmqtt_wincc.lib also contains
 * the rsmqtt client ABI (rsmqtt.h).
 *
 * Conventions:
 *   - names: NUL-terminated strings; payloads and values: (pointer, length)
 *   - rsmqtt_wincc_buf results are owned by the caller: rsmqtt_wincc_buf_free()
 *   - results kept in a handle stay valid until the next call that changes it
 *   - failures return -1 / NULL; rsmqtt_wincc_last_error() says why
 *
 * C++: include rsmqtt_wincc.hpp instead.
 */
#ifndef RSMQTT_WINCC_H
#define RSMQTT_WINCC_H

#include "rsmqtt.h"

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct rsmqtt_wincc_buf
{
  char  *data;  /* not NUL-terminated */
  size_t len;
} rsmqtt_wincc_buf;

void        rsmqtt_wincc_buf_free(rsmqtt_wincc_buf buf);
const char *rsmqtt_wincc_last_error(void);

/* ------------------------------------------------------------- addresses */

typedef struct RsmqttWinccAddress rsmqtt_wincc_address;

enum
{
  RSMQTT_WINCC_ADDR_CONNECTION = 0,
  RSMQTT_WINCC_ADDR_TOPIC      = 1,  /* Sparkplug: the edge node's subscription filter */
  RSMQTT_WINCC_ADDR_JSON_PATH  = 2,  /* as written */
  RSMQTT_WINCC_ADDR_TS_PATH    = 3,
  RSMQTT_WINCC_ADDR_SPB_GROUP  = 4,
  RSMQTT_WINCC_ADDR_SPB_NODE   = 5,
  RSMQTT_WINCC_ADDR_SPB_DEVICE = 6,
  RSMQTT_WINCC_ADDR_SPB_METRIC = 7
};

enum
{
  RSMQTT_WINCC_ADDR_QOS        = 0,  /* -1: connection default */
  RSMQTT_WINCC_ADDR_RETAIN     = 1,  /* -1: connection default */
  RSMQTT_WINCC_ADDR_SUB_FLAGS  = 2,
  RSMQTT_WINCC_ADDR_IS_STATE   = 3,
  RSMQTT_WINCC_ADDR_JSON       = 4,
  RSMQTT_WINCC_ADDR_HAS_TS     = 5,
  RSMQTT_WINCC_ADDR_SPARKPLUG  = 6,
  RSMQTT_WINCC_ADDR_SPB_ONLINE = 7
};

rsmqtt_wincc_address *rsmqtt_wincc_address_parse(const char *text, size_t len);
void        rsmqtt_wincc_address_free(rsmqtt_wincc_address *a);
const char *rsmqtt_wincc_address_text(const rsmqtt_wincc_address *a, int field);
int         rsmqtt_wincc_address_int(const rsmqtt_wincc_address *a, int field);
int         rsmqtt_wincc_topic_matches(const char *filter, const char *topic);
int         rsmqtt_wincc_is_wildcard_filter(const char *filter);

/* ------------------------------------------------------------------ JSON */

enum  /* value kinds */
{
  RSMQTT_WINCC_JSON_STRING = 0,
  RSMQTT_WINCC_JSON_NUMBER = 1,
  RSMQTT_WINCC_JSON_BOOL   = 2,
  RSMQTT_WINCC_JSON_NULL   = 3,
  RSMQTT_WINCC_JSON_OBJECT = 4,
  RSMQTT_WINCC_JSON_ARRAY  = 5
};

enum  /* lookup results */
{
  RSMQTT_WINCC_FOUND     = 0,
  RSMQTT_WINCC_MISSING   = 1,
  RSMQTT_WINCC_NOT_JSON  = 2
};

typedef struct RsmqttWinccExtractor rsmqtt_wincc_extractor;

rsmqtt_wincc_extractor *rsmqtt_wincc_extractor_new(void);
void        rsmqtt_wincc_extractor_free(rsmqtt_wincc_extractor *x);
void        rsmqtt_wincc_extractor_clear(rsmqtt_wincc_extractor *x);
int         rsmqtt_wincc_extractor_add(rsmqtt_wincc_extractor *x, const char *path, size_t len);
int         rsmqtt_wincc_extractor_size(const rsmqtt_wincc_extractor *x);
void        rsmqtt_wincc_extractor_run(rsmqtt_wincc_extractor *x, const char *doc, size_t len);
int         rsmqtt_wincc_extractor_result(const rsmqtt_wincc_extractor *x, int id, int *kind,
                                          const char **text, size_t *len);
const char *rsmqtt_wincc_extractor_error(const rsmqtt_wincc_extractor *x);

int rsmqtt_wincc_json_path_string(const char *path, size_t len, rsmqtt_wincc_buf *out);
int rsmqtt_wincc_json_quote(const char *text, size_t len, rsmqtt_wincc_buf *out);
int rsmqtt_wincc_json_build(size_t n, const char *const *paths, const size_t *path_lens,
                            const char *const *values, const size_t *value_lens, rsmqtt_wincc_buf *out);
int rsmqtt_wincc_json_parse_time(int kind, const char *text, size_t len, int64_t *ms);
int rsmqtt_wincc_json_iso_time(int64_t ms, rsmqtt_wincc_buf *out);

typedef struct RsmqttWinccItems rsmqtt_wincc_items;

rsmqtt_wincc_items *rsmqtt_wincc_dyn_items(const char *payload, size_t len);
size_t rsmqtt_wincc_items_count(const rsmqtt_wincc_items *items);
int    rsmqtt_wincc_items_get(const rsmqtt_wincc_items *items, size_t i, const char **text, size_t *len);
void   rsmqtt_wincc_items_free(rsmqtt_wincc_items *items);

/* ------------------------------------------------------------ redundancy */

int rsmqtt_wincc_redu_for_host(const char *base, int host2, rsmqtt_wincc_buf *out);
int rsmqtt_wincc_redu_ends_with_suffix(const char *name);
int rsmqtt_wincc_redu_split_option(const char *name, rsmqtt_wincc_buf *option);
int rsmqtt_wincc_redu_client_id(const char *client_id, int host2, int redu_given, rsmqtt_wincc_buf *out);

/* ---------------------------------------------------- Sparkplug B (host) */

enum
{
  RSMQTT_WINCC_SPB_VALUE  = 0,
  RSMQTT_WINCC_SPB_ONLINE = 1,
  RSMQTT_WINCC_SPB_STALE  = 2
};

typedef struct rsmqtt_wincc_spb_update
{
  int         kind;
  const char *group, *node, *device, *metric;  /* device "": the edge node; metric "" for ONLINE */
  const char *text;
  size_t      text_len;
  int         is_string;
  int         online;
  int         has_time;
  int64_t     time_ms;
} rsmqtt_wincc_spb_update;

typedef struct RsmqttWinccSpbHost rsmqtt_wincc_spb_host;

rsmqtt_wincc_spb_host *rsmqtt_wincc_spb_host_new(void);
void        rsmqtt_wincc_spb_host_free(rsmqtt_wincc_spb_host *h);
void        rsmqtt_wincc_spb_set_rebirth_interval(rsmqtt_wincc_spb_host *h, int64_t ms);
void        rsmqtt_wincc_spb_on_message(rsmqtt_wincc_spb_host *h, const char *topic, const char *payload,
                                        size_t len, int64_t now_ms);
void        rsmqtt_wincc_spb_connection_lost(rsmqtt_wincc_spb_host *h);
size_t      rsmqtt_wincc_spb_update_count(const rsmqtt_wincc_spb_host *h);
int         rsmqtt_wincc_spb_update_get(const rsmqtt_wincc_spb_host *h, size_t i, rsmqtt_wincc_spb_update *out);
size_t      rsmqtt_wincc_spb_publish_count(const rsmqtt_wincc_spb_host *h);
int         rsmqtt_wincc_spb_publish_get(const rsmqtt_wincc_spb_host *h, size_t i, const char **topic,
                                         const char **payload, size_t *len);
size_t      rsmqtt_wincc_spb_warning_count(const rsmqtt_wincc_spb_host *h);
const char *rsmqtt_wincc_spb_warning_get(const rsmqtt_wincc_spb_host *h, size_t i);
int         rsmqtt_wincc_spb_command(const rsmqtt_wincc_spb_host *h, const char *group, const char *node,
                                     const char *device, const char *metric, const char *value, size_t len,
                                     int64_t now_ms, rsmqtt_wincc_buf *topic, rsmqtt_wincc_buf *payload);
int         rsmqtt_wincc_spb_is_online(const rsmqtt_wincc_spb_host *h, const char *group, const char *node,
                                       const char *device);
uint32_t    rsmqtt_wincc_spb_data_type(const rsmqtt_wincc_spb_host *h, const char *group, const char *node,
                                       const char *device, const char *metric);
int         rsmqtt_wincc_spb_node_filter(const char *group, const char *node, rsmqtt_wincc_buf *out);
int         rsmqtt_wincc_spb_state_topic(const char *host_id, rsmqtt_wincc_buf *out);
int         rsmqtt_wincc_spb_state_payload(int online, int64_t timestamp_ms, rsmqtt_wincc_buf *out);
int         rsmqtt_wincc_spb_valid_id(const char *id);

/* ----------------------------------------------- connection settings */

typedef struct rsmqtt_wincc_str
{
  const char *data;  /* not NUL-terminated */
  size_t      len;
} rsmqtt_wincc_str;

/* The Config.* elements of a _RsMqttBroker DP (flags: 0 / 1). */
typedef struct rsmqtt_wincc_broker_config
{
  rsmqtt_wincc_str        server_uri;
  const rsmqtt_wincc_str *server_uris;
  size_t                  server_uris_len;
  rsmqtt_wincc_str        client_id;
  int                     mqtt_version, keep_alive, clean_session;
  rsmqtt_wincc_str        username, password, tls_trust_store, tls_key_store, tls_private_key;
  int                     tls_verify;
  rsmqtt_wincc_str        will_topic, will_payload;
  int                     will_qos, will_retain, subscribe_qos, publish_qos, publish_retain;
  const rsmqtt_wincc_str *options;  /* "name=value" */
  size_t                  options_len;
} rsmqtt_wincc_broker_config;

/* Decrypts a stored password: 0 and *out = the plain text, or -1 and *out =
 * why it failed; *out is allocated with rsmqtt_wincc_buf_new(). */
typedef int (*rsmqtt_wincc_reveal_fn)(void *ctx, const char *stored, size_t len, rsmqtt_wincc_buf *out);

enum
{
  RSMQTT_WINCC_SETTINGS_SUBSCRIBE_QOS         = 0,
  RSMQTT_WINCC_SETTINGS_PUBLISH_QOS           = 1,
  RSMQTT_WINCC_SETTINGS_PUBLISH_RETAIN        = 2,
  RSMQTT_WINCC_SETTINGS_RECONNECT_INTERVAL_MS = 3,
  RSMQTT_WINCC_SETTINGS_AUTO_RECONNECT        = 4,  /* read only */
  RSMQTT_WINCC_SETTINGS_STATE_TS              = 5   /* read only: timestamp of the STATE messages */
};

typedef struct RsmqttWinccSettings rsmqtt_wincc_settings;

rsmqtt_wincc_buf rsmqtt_wincc_buf_new(const char *data, size_t len);

rsmqtt_wincc_settings *rsmqtt_wincc_settings_new(const char *name, int man_num);
void    rsmqtt_wincc_settings_free(rsmqtt_wincc_settings *s);
int64_t rsmqtt_wincc_settings_get(const rsmqtt_wincc_settings *s, int field);
int     rsmqtt_wincc_settings_set(rsmqtt_wincc_settings *s, int field, int64_t value);
int     rsmqtt_wincc_settings_set_option(rsmqtt_wincc_settings *s, const char *name, const char *value, size_t len);
int     rsmqtt_wincc_settings_set_spb_host_id(rsmqtt_wincc_settings *s, const char *id);
int     rsmqtt_wincc_settings_apply_redu_option(rsmqtt_wincc_settings *s, const char *option, const char *value,
                                                size_t len, int host2);
int     rsmqtt_wincc_settings_apply_broker_config(rsmqtt_wincc_settings *s, const rsmqtt_wincc_broker_config *cfg,
                                                  int host2, rsmqtt_wincc_reveal_fn reveal, void *ctx);
int     rsmqtt_wincc_settings_spb_state_host(const rsmqtt_wincc_settings *s, int host2, rsmqtt_wincc_buf *out);
/* free the options with rsmqtt_options_free(); NULL on error */
rsmqtt_options *rsmqtt_wincc_settings_client_options(rsmqtt_wincc_settings *s, int host2, const char *data_dir,
                                                     int64_t now_ms, rsmqtt_wincc_buf *warnings);
int     rsmqtt_wincc_is_secret_option(const char *name);

/* ------------------------------------------------------- message router */

/* The addresses of one broker connection: subscriptions, $state addresses,
 * the Sparkplug host application. A message or a change of the connection
 * leaves its results (deliveries, publish, subscribe, warnings) in the router
 * until the next call that changes it. */
typedef struct RsmqttWinccRouter rsmqtt_wincc_router;

enum
{
  RSMQTT_WINCC_VALIDITY_KEEP    = 0,
  RSMQTT_WINCC_VALIDITY_VALID   = 1,
  RSMQTT_WINCC_VALIDITY_INVALID = 2
};

typedef struct rsmqtt_wincc_delivery
{
  const char *address;
  int         has_value;    /* 0: the payload of the message as it is */
  const char *value;
  size_t      value_len;
  int         has_time;     /* 0: now */
  int64_t     time_ms;
  int         periph_time;  /* the value's own time (ts= / Sparkplug) */
  int         validity;     /* RSMQTT_WINCC_VALIDITY_* */
} rsmqtt_wincc_delivery;

typedef struct rsmqtt_wincc_publish
{
  const char *topic;
  const char *payload;
  size_t      payload_len;
  int         qos;
  int         retain;
} rsmqtt_wincc_publish;

enum  /* rsmqtt_wincc_router_add */
{
  RSMQTT_WINCC_ADDED_NOTHING   = 0,
  RSMQTT_WINCC_ADDED_STATE     = 1,  /* a $state address: write the state */
  RSMQTT_WINCC_ADDED_SUBSCRIBE = 2   /* subscribe *filter */
};

rsmqtt_wincc_router *rsmqtt_wincc_router_new(void);
void   rsmqtt_wincc_router_free(rsmqtt_wincc_router *r);
int    rsmqtt_wincc_router_add(rsmqtt_wincc_router *r, const char *full_address, int is_dyn, int input, int default_qos,
                               rsmqtt_wincc_buf *filter);
int    rsmqtt_wincc_router_remove(rsmqtt_wincc_router *r, const char *full_address, int input, rsmqtt_wincc_buf *filter);
/* subscriptions of the driver's MSA service, by id: add 1 = subscribe *filter, always (-1 invalid filter),
   remove 1 = unsubscribe *filter; _service: the ids the last message is for, *text: UTF-8 without NUL */
int    rsmqtt_wincc_router_add_service(rsmqtt_wincc_router *r, uint64_t id, const char *filter, int qos,
                                       rsmqtt_wincc_buf *subscribe);
int    rsmqtt_wincc_router_remove_service(rsmqtt_wincc_router *r, uint64_t id, rsmqtt_wincc_buf *unsubscribe);
const uint64_t *rsmqtt_wincc_router_service(const rsmqtt_wincc_router *r, size_t *count, int *text);
int    rsmqtt_wincc_router_filter(const rsmqtt_wincc_router *r, const char *filter, int *qos, int *flags);
int    rsmqtt_wincc_router_filters(const rsmqtt_wincc_router *r, rsmqtt_wincc_buf *out);  /* each followed by NUL */
int    rsmqtt_wincc_router_has_addresses(const rsmqtt_wincc_router *r);
void   rsmqtt_wincc_router_message(rsmqtt_wincc_router *r, const char *topic, const char *payload, size_t len,
                                   int64_t now_ms, const char *state_host, int64_t state_ts, int retained);
void   rsmqtt_wincc_router_state(rsmqtt_wincc_router *r, int connected);
void   rsmqtt_wincc_router_connected(rsmqtt_wincc_router *r, const char *state_host, int64_t state_ts);
void   rsmqtt_wincc_router_connection_lost(rsmqtt_wincc_router *r);
size_t rsmqtt_wincc_router_delivery_count(const rsmqtt_wincc_router *r);
int    rsmqtt_wincc_router_delivery_get(const rsmqtt_wincc_router *r, size_t i, rsmqtt_wincc_delivery *out);
size_t rsmqtt_wincc_router_publish_count(const rsmqtt_wincc_router *r);
int    rsmqtt_wincc_router_publish_get(const rsmqtt_wincc_router *r, size_t i, rsmqtt_wincc_publish *out);
size_t rsmqtt_wincc_router_subscribe_count(const rsmqtt_wincc_router *r);
int    rsmqtt_wincc_router_subscribe_get(const rsmqtt_wincc_router *r, size_t i, const char **filter, int *qos);
size_t rsmqtt_wincc_router_warning_count(const rsmqtt_wincc_router *r);
const char *rsmqtt_wincc_router_warning_get(const rsmqtt_wincc_router *r, size_t i, int *sparkplug);
/* The message that writes `value` (payload text of data type value_type, 1001..1008) to an
 * output address; time_ms: the value's time (ts=); publish_qos / publish_retain: the defaults.
 * -1 and rsmqtt_wincc_last_error() if it cannot be written. */
int    rsmqtt_wincc_router_write(const rsmqtt_wincc_router *r, const char *full_address, const char *value,
                                 size_t len, int value_type, int64_t time_ms, int64_t now_ms, int publish_qos,
                                 int publish_retain, rsmqtt_wincc_buf *topic, rsmqtt_wincc_buf *payload, int *qos,
                                 int *retain);

/* ---------------------------------------------------------------- values */

/* the field of the value's type is set: i (int32, int64), u (uint32, uint64), f (float), b (bool) */
typedef struct rsmqtt_wincc_number
{
  int64_t  i;
  uint64_t u;
  double   f;
  int      b;
} rsmqtt_wincc_number;

/* A value of data type `type` (1002 int32 .. 1007 bool, _address.._datatype)
 * from payload text; json_string: the text came from a JSON string. */
int rsmqtt_wincc_parse_number(int type, const char *text, size_t len, int json_string, rsmqtt_wincc_number *out);

#ifdef __cplusplus
}
#endif

#endif
