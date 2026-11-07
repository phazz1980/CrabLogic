// Сгенерировано CrabLogic из проекта «blink_protected». Ручные правки будут перезаписаны.
// Порядок выполнения: blink → led

#include <Arduino.h>

// Выходы блоков
bool blink_out = false;

// blink: generator
uint32_t blink_start = 0;
bool blink_off_phase = false;
bool blink_idle = false;

void setup() {
  // led: digital_out
  pinMode(2, OUTPUT);
}

void loop() {
  // blink: generator
  if (true) {
    uint32_t blink_on = (uint32_t)(500UL);
    uint32_t blink_off = (uint32_t)(500UL);
    uint32_t blink_now = (uint32_t)millis();
    if (blink_idle) { blink_idle = false; blink_off_phase = false; blink_start = blink_now; }
    uint32_t blink_len = blink_off_phase ? blink_off : blink_on;
    uint32_t blink_d = blink_now - blink_start;
    if (blink_d >= blink_len) {
      if (blink_d - blink_len < blink_on + blink_off - blink_len) {
        blink_off_phase = !blink_off_phase;
        blink_start += blink_len;
      } else {
        uint32_t blink_period = blink_on + blink_off;
        if (blink_period == 0) blink_period = 1;
        uint32_t blink_r = blink_d % blink_period;
        if (blink_r < blink_len) {
          blink_start = blink_now - blink_r;
        } else {
          blink_off_phase = !blink_off_phase;
          blink_start = blink_now - (blink_r - blink_len);
        }
      }
    }
    blink_out = !blink_off_phase && blink_on != 0;
  } else {
    blink_idle = true;
    blink_out = false;
  }

  // led: digital_out
  digitalWrite(2, (blink_out) ? HIGH : LOW);
}
