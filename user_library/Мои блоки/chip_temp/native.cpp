// crab:native
// crab:sim temp=35 ok=true
// Internal die temperature (ESP32-C3, S2, S3, C6); the chip has no sensor on plain ESP32.
struct State {
  uint32_t last;
  uint32_t period;
  bool first;
  float temp;
  bool ok;
};

CRAB_INIT(State& s, Params p) {
  s.period = p.period;
  s.first = true;
}

CRAB_STEP(State& s, In in, Out& out) {
  if (s.first || millis() - s.last >= s.period) {
    s.first = false;
    s.last = millis();
#if defined(SOC_TEMP_SENSOR_SUPPORTED) && SOC_TEMP_SENSOR_SUPPORTED
    s.temp = temperatureRead();
    s.ok = true;
#else
    s.ok = false;
#endif
  }
  out.temp = s.temp;
  out.ok = s.ok;
}
