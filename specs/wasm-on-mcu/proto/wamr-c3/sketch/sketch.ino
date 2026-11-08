// See bench.cpp: the WAMR measurements run once at start-up.
void bench_setup();
void setup() { bench_setup(); }
void loop() { delay(1000); }
