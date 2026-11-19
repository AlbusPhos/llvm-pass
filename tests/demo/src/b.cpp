extern "C" int compute_cpp(int x) {
    int acc = 0;
    for (int i = 0; i < x; ++i) {
        acc += (i * 3) ^ 0x5A;
    }
    return acc;
}