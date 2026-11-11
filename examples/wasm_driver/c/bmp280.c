// BMP280 driver block in C (`specs/wasm-drivers`): the same as ../rust/src/lib.rs.
// ABI: help/12-drivers.md. Build: ..\build.ps1 c

#include <stdint.h>

#define IMPORT(mod, name) __attribute__((import_module(mod), import_name(name)))

IMPORT("crab", "param") double param(int index);
IMPORT("crab", "output") void output(int index, double value);
IMPORT("bus", "i2c_open") int i2c_open(int sda, int scl, int hz);
IMPORT("bus", "i2c_write") int i2c_write(int addr, const uint8_t *ptr, int len);
IMPORT("bus", "i2c_write_read")
int i2c_write_read(int addr, const uint8_t *wptr, int wlen, uint8_t *rptr, int rlen);

enum { P_ADDRESS, P_SDA, P_SCL, P_PERIOD };
enum { O_TEMPERATURE, O_PRESSURE, O_OK };

#define REG_CALIB 0x88
#define REG_ID 0xD0
#define REG_CTRL_MEAS 0xF4
#define REG_DATA 0xF7
#define CHIP_ID 0x58
#define FORCED_X1 0x25 /* temperature and pressure x1, forced mode */
#define MEASURE_MS 10

static int32_t t1, t2, t3;
static int64_t p[9];
static int connected, measuring;
static int32_t wait_ms;
static uint8_t buf[24];

static int addr(void) { return (int)param(P_ADDRESS); }

static int read_reg(uint8_t reg, int n) {
    uint8_t r = reg;
    return i2c_write_read(addr(), &r, 1, buf, n) == n;
}

static int write_reg(uint8_t reg, uint8_t value) {
    uint8_t w[2] = {reg, value};
    return i2c_write(addr(), w, 2) == 2;
}

static int32_t u16le(int at) { return buf[at] | buf[at + 1] << 8; }
static int32_t i16le(int at) { return (int16_t)(buf[at] | buf[at + 1] << 8); }

static int connect(void) {
    if (!read_reg(REG_ID, 1) || buf[0] != CHIP_ID || !read_reg(REG_CALIB, 24)) return 0;
    t1 = u16le(0);
    t2 = i16le(2);
    t3 = i16le(4);
    p[0] = u16le(6);
    for (int k = 1; k < 9; k++) p[k] = i16le(6 + 2 * k);
    return 1;
}

void init(void) {
    i2c_open((int)param(P_SDA), (int)param(P_SCL), 400000);
    connected = connect();
    wait_ms = 0;
    measuring = 0;
}

void step(int dt_ms) {
    wait_ms -= dt_ms;
    if (wait_ms > 0) return;
    int period = (int)param(P_PERIOD);
    if (!connected && !(connected = connect())) {
        output(O_OK, 0);
        wait_ms = period;
        return;
    }
    if (!measuring) {
        measuring = write_reg(REG_CTRL_MEAS, FORCED_X1);
        wait_ms = measuring ? MEASURE_MS : period;
        if (!measuring) output(O_OK, 0);
        return;
    }
    measuring = 0;
    wait_ms = period - MEASURE_MS;
    if (!read_reg(REG_DATA, 6)) {
        output(O_OK, 0);
        return;
    }
    int32_t adc_p = buf[0] << 12 | buf[1] << 4 | buf[2] >> 4;
    int32_t adc_t = buf[3] << 12 | buf[4] << 4 | buf[5] >> 4;

    int32_t var1 = (((adc_t >> 3) - (t1 << 1)) * t2) >> 11;
    int32_t d = (adc_t >> 4) - t1;
    int32_t var2 = (((d * d) >> 12) * t3) >> 14;
    int32_t t_fine = var1 + var2;
    output(O_TEMPERATURE, ((t_fine * 5 + 128) >> 8) / 100.0);

    int64_t v1 = (int64_t)t_fine - 128000;
    int64_t v2 = v1 * v1 * p[5];
    v2 += (v1 * p[4]) << 17;
    v2 += p[3] << 35;
    v1 = ((v1 * v1 * p[2]) >> 8) + ((v1 * p[1]) << 12);
    v1 = (((int64_t)1 << 47) + v1) * p[0] >> 33;
    if (v1 == 0) {
        output(O_OK, 0);
        return;
    }
    int64_t pr = 1048576 - adc_p;
    pr = (((pr << 31) - v2) * 3125) / v1;
    int64_t w1 = (p[8] * (pr >> 13) * (pr >> 13)) >> 25;
    int64_t w2 = (p[7] * pr) >> 19;
    pr = ((pr + w1 + w2) >> 8) + (p[6] << 4);
    output(O_PRESSURE, pr / 256.0 / 100.0);
    output(O_OK, 1);
}
