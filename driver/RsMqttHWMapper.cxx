#include "RsMqttHWMapper.hxx"
#include "RsMqttConnections.hxx"
#include "RsMqttTrans.hxx"

#include <ErrHdl.hxx>
#include <HWObject.hxx>
#include <PVSSMacros.hxx>

#include <string>

static void addrError(const CharString &text)
{
  ErrHdl::error(ErrClass::PRIO_SEVERE, ErrClass::ERR_PARAM, ErrClass::UNEXPECTEDSTATE,
                "RsMqttHWMapper", "addDpPa", text);
}

static bool isInput(const PeriphAddr *confPtr)
{
  return !confPtr->isOnlyOutput() && !confPtr->isAlert();
}

//--------------------------------------------------------------------------------

PVSSboolean RsMqttHWMapper::addDpPa(DpIdentifier &dpId, PeriphAddr *confPtr)
{
  const std::string name((const char *) confPtr->getName());
  DEBUG_DRV_USR1("addDpPa " << name.c_str() << " datatype " << (int) confPtr->getTransformationType());

  if ( !RsMqttTrans::isSupported(confPtr->getTransformationType()) )
  {
    addrError(CharString("address '") + name.c_str() + "': unsupported transformation type " +
              CharString((int) confPtr->getTransformationType()) + " (use 1000..1008)");
    return TreeHWMapper::addDpPa(dpId, confPtr, HWMapDpPa::TransformationError);
  }
  confPtr->setTransform(new RsMqttTrans(confPtr->getTransformationType()));

  RsMqttAddress addr;
  std::string err;
  if ( !RsMqttAddress::parse(name, addr, err) )
  {
    addrError(CharString("address '") + name.c_str() + "': " + err.c_str());
    return TreeHWMapper::addDpPa(dpId, confPtr, HWMapDpPa::AddressError);
  }

  // The connection may not be configured yet (broker DPs arrive later);
  // addAddress() then creates a placeholder.
  RsMqttConnections &conns = RsMqttConnections::instance();

  bool input = isInput(confPtr);
  if ( !input && (addr.isState || RsMqttAddress::isWildcardFilter(addr.topic)) )
  {
    addrError(CharString("address '") + name.c_str() + "': cannot publish to a topic filter or $state");
    return TreeHWMapper::addDpPa(dpId, confPtr, HWMapDpPa::AddressError);
  }

  if ( !TreeHWMapper::addDpPa(dpId, confPtr, HWMapDpPa::getInitialError(confPtr)) )
    return PVSS_FALSE;

  if ( conns.addAddress(name, addr, input) )
  {
    // first DPE with this address: create its HWObject
    HWObject *hwObj = new HWObject;
    hwObj->setConnectionId(confPtr->getConnectionId());
    hwObj->setAddress(name.c_str());
    hwObj->setType(confPtr->getTransform()->isA());
    hwObj->setDlen(confPtr->getTransform()->itemSize());
    addHWObject(hwObj);
  }
  return PVSS_TRUE;
}

//--------------------------------------------------------------------------------

PVSSboolean RsMqttHWMapper::clrDpPa(DpIdentifier &dpId, PeriphAddr *confPtr)
{
  const std::string name((const char *) confPtr->getName());
  DEBUG_DRV_USR1("clrDpPa " << name.c_str());

  RsMqttConnections &conns = RsMqttConnections::instance();
  if ( const RsMqttAddress *parsed = conns.parsed(name) )
  {
    RsMqttAddress addr = *parsed;  // copy: removeAddress may erase it
    if ( conns.removeAddress(name, addr, isInput(confPtr)) )
    {
      HWObject adrObj;
      adrObj.setAddress(name.c_str());
      if ( HWObject *hwObj = findHWAddr(&adrObj) )
      {
        clrHWObject(hwObj);
        delete hwObj;
      }
    }
  }

  return TreeHWMapper::clrDpPa(dpId, confPtr);
}
