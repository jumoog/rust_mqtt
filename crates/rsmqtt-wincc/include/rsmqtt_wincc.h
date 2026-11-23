/*
 * rsmqtt_wincc.h - C ABI of rsmqtt-wincc: the WinCC OA driver logic of rsmqtt
 * that does not need the WinCC OA API (address syntax, JSON paths, Sparkplug B
 * host application, redundancy names). Implemented in Rust
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

#ifdef __cplusplus
}
#endif

#endif
