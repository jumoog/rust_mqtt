// RsMqttTrans.hxx - conversion between WinCC OA values and MQTT payloads.
//
// To the peripheral (toPeriph) the buffer receives a 4 byte length
// (little endian) followed by the payload bytes, because text payloads must
// not depend on NUL termination and blobs may contain NUL bytes.
// From the peripheral (toVar) the buffer is the raw payload.
//
// dyn DPEs (dyn_string, dyn_int, dyn_float, dyn_bool, ...): the data type is
// the type of the items. Input takes a JSON array (one item per element) or
// a single value (one item); output is a JSON array.
#ifndef RSMQTT_TRANS_HXX
#define RSMQTT_TRANS_HXX

#include <Transformation.hxx>

#include <string>
#include <string_view>

class RsMqttTrans : public Transformation
{
  public:
    explicit RsMqttTrans(TransformationType type, bool dyn = false);

    static bool isSupported(TransformationType type);
    /// Data types a dyn DPE can have (all but blob).
    static bool isSupportedDyn(TransformationType type);

    TransformationType isA() const override;
    int itemSize() const override;
    VariableType getVariableType() const override;
    Transformation *clone() const override;

    bool isDyn() const { return dyn; }

    PVSSboolean toPeriph(PVSSchar *dataPtr, PVSSushort len, const Variable &var, const PVSSushort subix) const override;
    PVSSboolean toPeriph(PVSSchar *dataPtr, PVSSulong len, const Variable &var, const PVSSulong subix) const override;

    VariablePtr toVar(const PVSSchar *data, const PVSSushort dlen, const PVSSushort subix) const override;
    VariablePtr toVar(const PVSSchar *data, const PVSSulong dlen, const PVSSulong subix) const override;

  private:
    VariableType itemType() const;
    /// One value as payload text; json: as JSON (strings quoted).
    bool itemToText(const Variable &var, bool json, std::string &text) const;
    /// One value from payload text; jsonString: the text came from a JSON string.
    Variable *textToItem(std::string_view text, bool jsonString, std::string &why) const;
    VariablePtr toDynVar(std::string_view payload) const;

    TransformationType type;
    bool dyn;
};

#endif
