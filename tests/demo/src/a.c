int compute_c(int a, int b) {
    return (a ^ b) + (a & b) * 2;
}