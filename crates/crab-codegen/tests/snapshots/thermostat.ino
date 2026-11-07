// Сгенерировано CrabLogic из проекта «thermostat». Ручные правки будут перезаписаны.
// Порядок выполнения: adc1 → adc2 → t1 → t2 → sum → two → temp → heat → heater → setpoint → excess → gain → duty → fan → hot → blink → alarm

#include <Arduino.h>

// Выходы блоков
int32_t adc1_out = 0;
int32_t adc2_out = 0;
float t1_out = 0.0f;
float t2_out = 0.0f;
float sum_out = 0.0f;
float two_out = 0.0f;
float temp_out = 0.0f;
bool heat_out = false;
float setpoint_out = 0.0f;
float excess_out = 0.0f;
float gain_out = 0.0f;
float duty_out = 0.0f;
bool hot_out = false;
bool blink_out = false;

// blink: generator
uint32_t blink_start = 0;
bool blink_off_phase = false;
bool blink_idle = false;

// pwm_out: общий код
static inline int32_t crab_f2i(float x) {
  if (x != x) return 0;
  if (x >= 2147483648.0f) return INT32_MAX;
  if (x <= -2147483648.0f) return INT32_MIN;
  return (int32_t)x;
}

void setup() {
  // two: constant
  two_out = 2.0f;

  // heater: digital_out
  pinMode(26, OUTPUT);

  // setpoint: constant
  setpoint_out = 22.0f;

  // gain: constant
  gain_out = 10.0f;

  // alarm: digital_out
  pinMode(27, OUTPUT);
}

void loop() {
  // adc1: analog_in
  adc1_out = analogRead(34);

  // adc2: analog_in
  adc2_out = analogRead(35);

  // t1: scale
  t1_out = 0.0f + ((float)adc1_out - 0.0f) * (50.0f - 0.0f) / (4095.0f - 0.0f);
  t1_out = constrain(t1_out, 0.0f, 50.0f);

  // t2: scale
  t2_out = 0.0f + ((float)adc2_out - 0.0f) * (50.0f - 0.0f) / (4095.0f - 0.0f);
  t2_out = constrain(t2_out, 0.0f, 50.0f);

  // sum: add
  sum_out = t1_out + t2_out;

  // temp: div
  temp_out = two_out != 0.0f ? sum_out / two_out : 0.0f;

  // heat: hysteresis
  if (temp_out <= 18.0f) {
    heat_out = true;
  } else if (temp_out >= 21.0f) {
    heat_out = false;
  }

  // heater: digital_out
  digitalWrite(26, (heat_out) ? HIGH : LOW);

  // excess: sub
  excess_out = temp_out - setpoint_out;

  // duty: mul
  duty_out = excess_out * gain_out;

  // fan: pwm_out
  analogWrite(25, crab_f2i(constrain(duty_out, 0.0f, 100.0f) * 2.55f + 0.5f));

  // hot: greater
  hot_out = temp_out > 30.0f;

  // blink: generator
  if (hot_out) {
    uint32_t blink_on = (uint32_t)(200UL);
    uint32_t blink_off = (uint32_t)(300UL);
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

  // alarm: digital_out
  digitalWrite(27, (blink_out) ? HIGH : LOW);
}
