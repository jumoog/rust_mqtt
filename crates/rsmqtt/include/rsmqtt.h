/*
 * rsmqtt.h - C ABI of the rsmqtt MQTT client library.
 *
 * rsmqtt wraps the pure-Rust Eclipse Paho MQTT client and offers the feature
 * set of Paho C's MQTTAsync: MQTT 3.1 / 3.1.1 / 5, QoS 0-2, tcp:// mqtt://
 * ssl:// tls:// mqtts:// ws:// wss://, HTTP(S) proxies, last will, automatic
 * reconnect, offline buffering, several server URIs, file/memory persistence
 * and MQTT v5 properties / reason codes / subscribe options.
 *
 * Model
 * -----
 *  - Settings are name/value strings (rsmqtt_options_set). Names are the
 *    field names of Paho C's option structs, case-insensitive, e.g.
 *    "serverURI", "clientId", "MQTTVersion", "keepAliveInterval",
 *    "cleansession", "automaticReconnect", "will.topic", "ssl.trustStore".
 *  - Operations (connect, publish, subscribe, ...) return immediately with a
 *    token. Their results, incoming messages and connection state changes are
 *    queued and fetched with rsmqtt_poll_event() - no callbacks into the host
 *    from library threads (except the optional notify/trace callbacks).
 *  - All functions are thread-safe. Return codes: 0 = success, < 0 = error;
 *    rsmqtt_last_error() gives the error text of the calling thread.
 *
 * For C++ use rsmqtt.hpp, a header-only RAII wrapper of this API.
 */
#ifndef RSMQTT_H
#define RSMQTT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------- handles */

typedef struct rsmqtt_options    rsmqtt_options;
typedef struct rsmqtt_properties rsmqtt_properties;
typedef struct rsmqtt_client     rsmqtt_client;
typedef struct rsmqtt_event      rsmqtt_event;
typedef uint64_t                 rsmqtt_token;

/* ----------------------------------------------------------- return codes */
/* -1 .. -18 have the values and meaning of Paho C's MQTTASYNC_* codes.     */

#define RSMQTT_SUCCESS                  0
#define RSMQTT_FAILURE                 -1
#define RSMQTT_PERSISTENCE_ERROR       -2
#define RSMQTT_DISCONNECTED            -3
#define RSMQTT_MAX_MESSAGES_INFLIGHT   -4
#define RSMQTT_BAD_UTF8_STRING         -5
#define RSMQTT_NULL_PARAMETER          -6
#define RSMQTT_BAD_STRUCTURE           -8
#define RSMQTT_BAD_QOS                 -9
#define RSMQTT_OPERATION_INCOMPLETE   -11
#define RSMQTT_MAX_BUFFERED_MESSAGES  -12
#define RSMQTT_SSL_NOT_SUPPORTED      -13
#define RSMQTT_BAD_PROTOCOL           -14
#define RSMQTT_BAD_MQTT_OPTION        -15
#define RSMQTT_WRONG_MQTT_VERSION     -16
#define RSMQTT_ZERO_LEN_WILL_TOPIC    -17
#define RSMQTT_COMMAND_IGNORED        -18

#define RSMQTT_TIMEOUT               -100
#define RSMQTT_BAD_TOPIC             -101
#define RSMQTT_TLS_ERROR             -102
#define RSMQTT_IO_ERROR              -103
#define RSMQTT_REASON_CODE           -104 /* server answered with an error reason code */
#define RSMQTT_PROTOCOL_ERROR        -105
#define RSMQTT_ALREADY_CONNECTED     -106
#define RSMQTT_CONNECT_IN_PROGRESS   -107
#define RSMQTT_PROXY_ERROR           -108
#define RSMQTT_WEBSOCKET_ERROR       -109
#define RSMQTT_INVALID_ARGUMENT      -110
#define RSMQTT_SESSION_NOT_RESUMED   -111
#define RSMQTT_UNKNOWN_OPTION        -112
#define RSMQTT_PANIC                 -199

/* ------------------------------------------------------------ event types */

#define RSMQTT_EVENT_CONNECTED        1  /* (re)connected; session_present, server_uri, properties */
#define RSMQTT_EVENT_CONNECTION_LOST  2  /* connection dropped (auto reconnect continues if enabled) */
#define RSMQTT_EVENT_DISCONNECTED     3  /* server sent DISCONNECT (v5); reason_code, properties */
#define RSMQTT_EVENT_MESSAGE          4  /* incoming message: topic, payload, qos, retained, dup, properties */
#define RSMQTT_EVENT_COMPLETE         5  /* an operation finished: token, op, rc, reason code(s) */

/* operation of a COMPLETE event */
#define RSMQTT_OP_CONNECT             1
#define RSMQTT_OP_PUBLISH             2
#define RSMQTT_OP_SUBSCRIBE           3
#define RSMQTT_OP_UNSUBSCRIBE         4
#define RSMQTT_OP_DISCONNECT          5

/* subscribe flags (MQTT v5 subscription options, wire format bit positions) */
#define RSMQTT_SUB_NO_LOCAL             0x04
#define RSMQTT_SUB_RETAIN_AS_PUBLISHED  0x08
#define RSMQTT_SUB_RETAIN_SEND_ON_NEW   0x10 /* retain handling 1 */
#define RSMQTT_SUB_RETAIN_DONT_SEND     0x20 /* retain handling 2 */

/* property list selectors for rsmqtt_options_set_properties */
#define RSMQTT_PROPS_CONNECT          1
#define RSMQTT_PROPS_WILL             2

/* disconnect flags */
#define RSMQTT_DISCONNECT_PUBLISH_WILL 0x01

/* MQTT v5 property identifiers */
#define RSMQTT_PROP_PAYLOAD_FORMAT_INDICATOR        0x01
#define RSMQTT_PROP_MESSAGE_EXPIRY_INTERVAL         0x02
#define RSMQTT_PROP_CONTENT_TYPE                    0x03
#define RSMQTT_PROP_RESPONSE_TOPIC                  0x08
#define RSMQTT_PROP_CORRELATION_DATA                0x09
#define RSMQTT_PROP_SUBSCRIPTION_IDENTIFIER         0x0B
#define RSMQTT_PROP_SESSION_EXPIRY_INTERVAL         0x11
#define RSMQTT_PROP_ASSIGNED_CLIENT_IDENTIFIER      0x12
#define RSMQTT_PROP_SERVER_KEEP_ALIVE               0x13
#define RSMQTT_PROP_AUTHENTICATION_METHOD           0x15
#define RSMQTT_PROP_AUTHENTICATION_DATA             0x16
#define RSMQTT_PROP_REQUEST_PROBLEM_INFORMATION     0x17
#define RSMQTT_PROP_WILL_DELAY_INTERVAL             0x18
#define RSMQTT_PROP_REQUEST_RESPONSE_INFORMATION    0x19
#define RSMQTT_PROP_RESPONSE_INFORMATION            0x1A
#define RSMQTT_PROP_SERVER_REFERENCE                0x1C
#define RSMQTT_PROP_REASON_STRING                   0x1F
#define RSMQTT_PROP_RECEIVE_MAXIMUM                 0x21
#define RSMQTT_PROP_TOPIC_ALIAS_MAXIMUM             0x22
#define RSMQTT_PROP_TOPIC_ALIAS                     0x23
#define RSMQTT_PROP_MAXIMUM_QOS                     0x24
#define RSMQTT_PROP_RETAIN_AVAILABLE                0x25
#define RSMQTT_PROP_USER_PROPERTY                   0x26
#define RSMQTT_PROP_MAXIMUM_PACKET_SIZE             0x27
#define RSMQTT_PROP_WILDCARD_SUBSCRIPTION_AVAILABLE 0x28
#define RSMQTT_PROP_SUBSCRIPTION_IDENTIFIERS_AVAILABLE 0x29
#define RSMQTT_PROP_SHARED_SUBSCRIPTION_AVAILABLE   0x2A

/* one property as returned by rsmqtt_properties_get (pointers borrowed) */
typedef struct rsmqtt_property {
    int         id;
    uint32_t    int_value;  /* integer properties */
    const char *data;       /* string/binary value or user property value; NOT NUL-terminated */
    size_t      data_len;
    const char *name;       /* user property name; NOT NUL-terminated; NULL otherwise */
    size_t      name_len;
} rsmqtt_property;

/* callbacks (called from library threads!) */
typedef void (*rsmqtt_notify_fn)(void *ctx);
typedef void (*rsmqtt_trace_fn)(int level, const char *message, void *ctx);

/* --------------------------------------------------------------- library */

const char *rsmqtt_version(void);
const char *rsmqtt_strerror(int rc);
const char *rsmqtt_reason_code_string(int reason_code);
const char *rsmqtt_last_error(void);
/* level: 0=off 1=error 2=warn 3=info 4=debug 5=trace */
void        rsmqtt_set_trace_callback(int level, rsmqtt_trace_fn cb, void *ctx);

/* --------------------------------------------------------------- options */
/*
 * Create options:  serverURI clientId MQTTVersion(0|3|4|5|3.1|3.1.1|5)
 *                  persistence(none|memory|file) persistenceDir
 *                  (persistenceDir alone selects file; an explicit
 *                  persistence= wins, whatever the order)
 *                  sendWhileDisconnected maxBufferedMessages(>= 4)
 *                  allowDisconnectedSendAtAnyTime deleteOldestMessages
 *                  restoreMessages persistQoS0 runtimeThreads
 *                  completionEvents(all|errors|none) maxQueuedEvents
 * Connect options: keepAliveInterval cleansession cleanstart maxInflight
 *                  username password connectTimeout serverURIs(list)
 *                  automaticReconnect minRetryInterval maxRetryInterval
 *                  httpProxy httpsProxy httpHeader("Name: value", repeatable)
 * v5 connect:      sessionExpiryInterval receiveMaximum maximumPacketSize
 *                  topicAliasMaximum requestResponseInformation
 *                  requestProblemInformation userProperty("k=v", repeatable)
 * Will:            will.topic will.payload will.qos will.retained
 *                  will.delayInterval will.messageExpiryInterval
 *                  will.contentType will.responseTopic
 *                  will.payloadFormatIndicator will.userProperty
 * v5 enhanced authentication: auth.method auth.data (see rsmqtt_auth_callback)
 * TLS (ssl.|tls.): ssl.trustStore ssl.keyStore ssl.privateKey
 *                  ssl.privateKeyPassword ssl.enabledCipherSuites
 *                  ssl.enableServerCertAuth ssl.verify ssl.CApath
 *                  ssl.disableDefaultTrustStore ssl.sslVersion ssl.alpn
 * Durations are seconds (fractions and ms/s/min/h suffixes accepted);
 * booleans are 1/0/true/false/yes/no/on/off.
 */
rsmqtt_options *rsmqtt_options_new(void);
rsmqtt_options *rsmqtt_options_clone(const rsmqtt_options *opts);
void            rsmqtt_options_free(rsmqtt_options *opts);
int             rsmqtt_options_set(rsmqtt_options *opts, const char *name, const char *value);
/* binary values: "password", "auth.data", "will.payload", "ssl.alpn" (ALPN wire format) */
int             rsmqtt_options_set_binary(rsmqtt_options *opts, const char *name,
                                          const void *data, size_t len);
int             rsmqtt_options_set_properties(rsmqtt_options *opts, int which,
                                              const rsmqtt_properties *props);
int             rsmqtt_option_is_known(const char *name);

/* MQTT v5 enhanced authentication (AUTH packets, needs MQTTVersion 5).
 *
 * auth.method = SCRAM-SHA-1 | SCRAM-SHA-256 | SCRAM-SHA-512 is built in: the
 * credentials are "username" and "password" (the password is not sent), and
 * the connect fails if the broker's CONNACK does not prove that it knows the
 * password. Other methods need a callback; it answers every method, the
 * built-in ones included, once it is set. "auth.data" is the authentication
 * data of CONNECT (for a callback's method; text, or binary with
 * rsmqtt_options_set_binary).
 *
 * The callback gets each AUTH challenge of the broker (method, data) and
 * answers with rsmqtt_auth_reply_set(reply, ...) and 0; anything else
 * abandons the connect. It is called on a library thread while the connect
 * waits (the one exception to "no calls from library threads"): answer
 * quickly and do not call into the client. */
typedef struct rsmqtt_auth_reply rsmqtt_auth_reply;
typedef int (*rsmqtt_auth_callback)(void *ctx, const char *method, const void *data, size_t len,
                                    rsmqtt_auth_reply *reply);
int             rsmqtt_options_set_auth_callback(rsmqtt_options *opts, rsmqtt_auth_callback cb, void *ctx);
int             rsmqtt_auth_reply_set(rsmqtt_auth_reply *reply, const void *data, size_t len);

/* ------------------------------------------------------------ properties */

rsmqtt_properties *rsmqtt_properties_new(void);
rsmqtt_properties *rsmqtt_properties_clone(const rsmqtt_properties *props);
void               rsmqtt_properties_free(rsmqtt_properties *props);
void               rsmqtt_properties_clear(rsmqtt_properties *props);
size_t             rsmqtt_properties_count(const rsmqtt_properties *props);
int                rsmqtt_properties_add_int(rsmqtt_properties *props, int id, uint32_t value);
int                rsmqtt_properties_add_string(rsmqtt_properties *props, int id, const char *value);
int                rsmqtt_properties_add_binary(rsmqtt_properties *props, int id,
                                                const void *data, size_t len);
int                rsmqtt_properties_add_user(rsmqtt_properties *props, const char *name,
                                              const char *value);
int                rsmqtt_properties_get(const rsmqtt_properties *props, size_t index,
                                         rsmqtt_property *out);
/* copies the value (NUL-terminated) and returns its length, -1 if absent */
ptrdiff_t          rsmqtt_properties_get_user(const rsmqtt_properties *props, const char *name,
                                              char *buf, size_t buf_len);

/* ---------------------------------------------------------------- client */

int  rsmqtt_client_create(const rsmqtt_options *opts, rsmqtt_client **out);
void rsmqtt_client_destroy(rsmqtt_client *client);
/* new connect options for the next rsmqtt_connect (create options ignored) */
int  rsmqtt_client_set_options(rsmqtt_client *client, const rsmqtt_options *opts);
/* cb is called (from a library thread) when the event queue becomes non-empty */
int  rsmqtt_set_event_notify(rsmqtt_client *client, rsmqtt_notify_fn cb, void *ctx);

int  rsmqtt_connect(rsmqtt_client *client, rsmqtt_token *token);
int  rsmqtt_reconnect(rsmqtt_client *client, rsmqtt_token *token);
/* reason_code < 0: normal disconnection; flags: RSMQTT_DISCONNECT_* */
int  rsmqtt_disconnect(rsmqtt_client *client, int timeout_ms, int reason_code, int flags,
                       const rsmqtt_properties *props, rsmqtt_token *token);
int  rsmqtt_is_connected(const rsmqtt_client *client);

int  rsmqtt_publish(rsmqtt_client *client, const char *topic, const void *payload, size_t len,
                    int qos, int retained, const rsmqtt_properties *props, rsmqtt_token *token);
int  rsmqtt_subscribe(rsmqtt_client *client, const char *filter, int qos, int flags,
                      const rsmqtt_properties *props, rsmqtt_token *token);
/* flags may be NULL */
int  rsmqtt_subscribe_many(rsmqtt_client *client, size_t count, const char *const *filters,
                           const int *qos, const int *flags, const rsmqtt_properties *props,
                           rsmqtt_token *token);
int  rsmqtt_unsubscribe(rsmqtt_client *client, const char *filter,
                        const rsmqtt_properties *props, rsmqtt_token *token);
int  rsmqtt_unsubscribe_many(rsmqtt_client *client, size_t count, const char *const *filters,
                             const rsmqtt_properties *props, rsmqtt_token *token);

/* blocks until the operation completed and returns its result code */
int  rsmqtt_wait_for_completion(rsmqtt_client *client, rsmqtt_token token, int timeout_ms);
int  rsmqtt_get_pending_tokens(rsmqtt_client *client, rsmqtt_token **tokens, size_t *count);
void rsmqtt_free_tokens(rsmqtt_token *tokens, size_t count);

size_t    rsmqtt_queued_events(const rsmqtt_client *client);
uint64_t  rsmqtt_dropped_events(const rsmqtt_client *client);
ptrdiff_t rsmqtt_client_id(const rsmqtt_client *client, char *buf, size_t buf_len);

/* ---------------------------------------------------------------- events */

/* timeout_ms: 0 = don't wait, < 0 = forever. Returns 1 = *ev set, 0 = none, < 0 error */
int  rsmqtt_poll_event(rsmqtt_client *client, int timeout_ms, rsmqtt_event **ev);
void rsmqtt_event_free(rsmqtt_event *ev);

int          rsmqtt_event_type(const rsmqtt_event *ev);
int          rsmqtt_event_op(const rsmqtt_event *ev);
rsmqtt_token rsmqtt_event_token(const rsmqtt_event *ev);
int          rsmqtt_event_rc(const rsmqtt_event *ev);
int          rsmqtt_event_reason_code(const rsmqtt_event *ev);
size_t       rsmqtt_event_reason_code_count(const rsmqtt_event *ev);
int          rsmqtt_event_reason_code_at(const rsmqtt_event *ev, size_t index);
int          rsmqtt_event_session_present(const rsmqtt_event *ev);
int          rsmqtt_event_mqtt_version(const rsmqtt_event *ev);
const char  *rsmqtt_event_server_uri(const rsmqtt_event *ev);
const char  *rsmqtt_event_text(const rsmqtt_event *ev);
const char  *rsmqtt_event_topic(const rsmqtt_event *ev, size_t *len);
const void  *rsmqtt_event_payload(const rsmqtt_event *ev, size_t *len);
int          rsmqtt_event_qos(const rsmqtt_event *ev);
int          rsmqtt_event_retained(const rsmqtt_event *ev);
int          rsmqtt_event_dup(const rsmqtt_event *ev);
int          rsmqtt_event_msgid(const rsmqtt_event *ev);
/* borrowed: valid until rsmqtt_event_free */
const rsmqtt_properties *rsmqtt_event_properties(const rsmqtt_event *ev);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* RSMQTT_H */
