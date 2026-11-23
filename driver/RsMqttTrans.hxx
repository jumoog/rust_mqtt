// RsMqttTrans.hxx - conversion between WinCC OA values and MQTT payloads.
//
// To the peripheral (toPeriph) the buffer receives a 4 byte length
// (little endian) followed by the payload bytes, because text payloads must
// not depend on NUL termination and blobs may contain NUL bytes.
// From the peripheral (toVar) the buffer is the raw payload.
#ifndef RSMQTT_TRANS_HXX
#define RSMQTT_TRANS_HXX

#include <Transformation.hxx>

class RsMqttTrans : public Transformation
{
  public:
    explicit RsMqttTrans(TransformationType type);

    static bool isSupported(TransformationType type);

    TransformationType isA() const override;
    int itemSize() const override;
    VariableType getVariableType() const override;
    Transformation *clone() const override;

    PVSSboolean toPeriph(PVSSchar *dataPtr, PVSSushort len, const Variable &var, const PVSSushort subix) const override;
    PVSSboolean toPeriph(PVSSchar *dataPtr, PVSSulong len, const Variable &var, const PVSSulong subix) const override;

    VariablePtr toVar(const PVSSchar *data, const PVSSushort dlen, const PVSSushort subix) const override;
    VariablePtr toVar(const PVSSchar *data, const PVSSulong dlen, const PVSSulong subix) const override;

  private:
    TransformationType type;
};

#endif
