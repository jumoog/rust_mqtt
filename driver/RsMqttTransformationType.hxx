#ifndef RSMQTT_TRANSFORMATIONTYPE_HXX
#define RSMQTT_TRANSFORMATIONTYPE_HXX

#include <ConfigTypes.hxx>

// Values for _address.._datatype of rsmqtt addresses.
// Payloads are text (UTF-8) except for Blob, which is the raw payload.
namespace RsMqttTransformationType
{
  enum : TransformationType
  {
    Undefined = TransUserType,  // 1000: treated as String
    String,                     // 1001: payload as text
    Int32,                      // 1002
    UInt32,                     // 1003
    Int64,                      // 1004
    UInt64,                     // 1005
    Float,                      // 1006: double
    Bool,                       // 1007: true/false, 1/0, on/off, yes/no
    Blob,                       // 1008: raw bytes
  };
}

#endif
