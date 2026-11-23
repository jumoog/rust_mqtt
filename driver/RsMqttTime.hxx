// RsMqttTime.hxx - times in ms since 1970 and WinCC OA TimeVars.
#ifndef RSMQTT_TIME_HXX
#define RSMQTT_TIME_HXX

#include <TimeVar.hxx>

#include <chrono>
#include <cstdint>

inline int64_t rsmqttNowMs()
{
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now().time_since_epoch()).count();
}

inline TimeVar rsmqttTimeOfMs(int64_t ms)
{
  long long sec = ms >= 0 ? ms / 1000 : -((-ms + 999) / 1000);
  return TimeVar((time_t) sec, (PVSSshort) (ms - sec * 1000));
}

#endif
