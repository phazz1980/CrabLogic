// Сгенерировано CrabLogic из проекта «text_commands». Ручные правки будут перезаписаны.
// Порядок выполнения: adc → bar → p_text → name → tick → p_send → port2 → rx → command_w → command_r → message → message_w → length → length_w → report → is_on → pump → head → is_set → arg → number → apply → setpoint_w

#include <Arduino.h>

// Выходы блоков
int32_t adc_out = 0;
float bar_out = 0.0f;
String p_text_out = String();
String name_out = String();
bool tick_out = false;
bool p_send_sent = false;
int32_t port2_baud = 0;
String rx_line = String();
bool rx_ready = false;
int32_t rx_available = 0;
String command_r_out = String();
String message_out = String();
int32_t length_out = 0;
bool report_sent = false;
bool is_on_out = false;
String head_out = String();
bool is_set_out = false;
String arg_out = String();
float number_out = 0.0f;
bool number_ok = false;
bool apply_out = false;

// Переменные
String var_message = String();
int32_t var_message_len = 0;
String var_command = String("SET 2.5");
float var_setpoint = 3.0f;

// tick: generator
uint32_t tick_start = 0;
bool tick_off_phase = false;
bool tick_idle = false;

// p_send: uart_send
float p_send_last;
bool p_send_started = false;

// rx: uart_read
String rx_buf;
unsigned long rx_last = 0;

// report: uart_send
bool report_prev = false;

// text_compare: общий код
static inline char crab_lower(char c) {
  return (c >= 'A' && c <= 'Z') ? (char)(c + 32) : c;
}
bool crab_text_match_nocase(const String& a, const String& b, int mode) {
  int la = a.length(), lb = b.length();
  if (lb > la || (mode == 0 && lb != la)) {
    return false;
  }
  int first = mode == 3 ? la - lb : 0;
  int last = (mode == 1 ? la : first + lb) - lb;
  for (int at = first; at <= last; at++) {
    int i = 0;
    while (i < lb && crab_lower(a[at + i]) == crab_lower(b[i])) {
      i++;
    }
    if (i == lb) {
      return true;
    }
  }
  return false;
}

// text_to_num: общий код
// Number from text: spaces around are skipped, a decimal comma is accepted.
bool crab_text_to_float(const String& text, float& out) {
  String t = text;
  t.trim();
  t.replace(',', '.');
  if (t.length() == 0) {
    return false;
  }
  char* end = nullptr;
  double v = strtod(t.c_str(), &end);
  if (end == t.c_str() || *end != '\0') {
    return false;
  }
  out = (float)v;
  return true;
}

void setup() {
  // USB: 115200 бод
  Serial.begin(115200);
  // UART2: 9600 бод, 8N1, RX GPIO16, TX GPIO17
  Serial2.setRxBufferSize(256);
  Serial2.begin(9600, SERIAL_8N1, 16, 17);

  // Память под строки заранее: меньше дробления кучи.
  p_text_out.reserve(20);
  name_out.reserve(10);
  rx_line.reserve(32);
  command_r_out.reserve(32);
  message_out.reserve(32);
  head_out.reserve(32);
  arg_out.reserve(32);
  var_message.reserve(32);
  var_command.reserve(32);

  // name: text_const
  name_out = "насос";

  // rx: uart_read
  rx_buf.reserve(64);

  // pump: digital_out
  pinMode(26, OUTPUT);
}

void loop() {
  // adc: analog_in
  adc_out = analogRead(34);

  // bar: scale
  bar_out = 0.0f + ((float)adc_out - 0.0f) * (10.0f - 0.0f) / (4095.0f - 0.0f);
  bar_out = constrain(bar_out, 0.0f, 10.0f);

  // p_text: num_to_text
  p_text_out = "P=";
  p_text_out += String(bar_out, 1);
  p_text_out += " бар";

  // tick: generator
  if (true) {
    uint32_t tick_on = (uint32_t)(500UL);
    uint32_t tick_off = (uint32_t)(500UL);
    uint32_t tick_now = (uint32_t)millis();
    if (tick_idle) { tick_idle = false; tick_off_phase = false; tick_start = tick_now; }
    uint32_t tick_len = tick_off_phase ? tick_off : tick_on;
    uint32_t tick_d = tick_now - tick_start;
    if (tick_d >= tick_len) {
      if (tick_d - tick_len < tick_on + tick_off - tick_len) {
        tick_off_phase = !tick_off_phase;
        tick_start += tick_len;
      } else {
        uint32_t tick_period = tick_on + tick_off;
        if (tick_period == 0) tick_period = 1;
        uint32_t tick_r = tick_d % tick_period;
        if (tick_r < tick_len) {
          tick_start = tick_now - tick_r;
        } else {
          tick_off_phase = !tick_off_phase;
          tick_start = tick_now - (tick_r - tick_len);
        }
      }
    }
    tick_out = !tick_off_phase && tick_on != 0;
  } else {
    tick_idle = true;
    tick_out = false;
  }

  // p_send: uart_send
  {
    // On change; the first value after start goes out too.
    bool changed = !p_send_started || fabsf(bar_out - p_send_last) >= 0.05f;
    bool fire = true && changed;
    p_send_sent = false;
    if (fire) {
      float data = bar_out;
      if ((size_t)Serial2.availableForWrite() >= sizeof(data)) {
        Serial2.write((const uint8_t*)&data, sizeof(data));
        p_send_sent = true;
      }
      if (p_send_sent) {
        p_send_last = bar_out;
        p_send_started = true;
      }
    }
  }

  // port2: uart_config
  port2_baud = (int32_t)Serial2.baudRate();

  // rx: uart_read
  rx_ready = false;
  // One line per cycle: the next one waits in the UART buffer.
  while (Serial2.available() > 0) {
    char c = (char)Serial2.read();
    rx_last = millis();
    if (c == '\r') {
      continue;
    }
    bool end = c == '\n';
    if (!end) {
      rx_buf += c;
      end = rx_buf.length() >= 64;
    }
    if (end) {
      rx_line = rx_buf;
      rx_buf = "";
      rx_ready = true;
      break;
    }
  }
  rx_available = Serial2.available();

  // command_w: var_set
  if (rx_ready) {
    var_command = rx_line;
  }

  // command_r: var_get
  command_r_out = var_command;

  // message: text_concat
  message_out = name_out;
  message_out += "; ";
  message_out += p_text_out;
  message_out += "; ";
  message_out += command_r_out;

  // message_w: var_set
  var_message = message_out;

  // length: text_length
  length_out = (int32_t)message_out.length();

  // length_w: var_set
  var_message_len = length_out;

  // report: uart_send
  {
    bool fire = tick_out && !report_prev;
    report_prev = tick_out;
    report_sent = false;
    if (fire) {
      const String& text = message_out;
      const char* ending = "\r\n";
      // A busy port would stall the loop: drop the message if the transmit buffer
      // is (nearly) full. A long one may still wait while the hardware FIFO drains.
      size_t room = text.length() + strlen(ending);
      if ((size_t)Serial.availableForWrite() >= (room < 64 ? room : 64)) {
        Serial.print(text);
        Serial.print(ending);
        report_sent = true;
      }
    }
  }

  // is_on: text_compare
  is_on_out = crab_text_match_nocase(command_r_out, "ON", 0);

  // pump: digital_out
  digitalWrite(26, (is_on_out) ? HIGH : LOW);

  // head: text_substring
  head_out = command_r_out.substring(0, 0 + 4);

  // is_set: text_compare
  is_set_out = head_out.startsWith("SET ");

  // arg: text_substring
  arg_out = command_r_out.substring(4);

  // number: text_to_num
  {
    float v;
    number_ok = crab_text_to_float(arg_out, v);
    if (number_ok) {
      number_out = v;
    }
  }

  // apply: and
  apply_out = is_set_out && number_ok;

  // setpoint_w: var_set
  if (apply_out) {
    var_setpoint = number_out;
  }
}
