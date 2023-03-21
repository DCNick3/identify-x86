#include <stdio.h>

void abs_inplace(int* i) {
    int v = *i;   // get the value
    if (v < 0)    // if it's negative
        v = -v;   // make it positive
    *i = v; // write it back
}

int main(void)
{
    int i = 42;
    abs_inplace(&i);
    printf("%d", i);
    return 0;
}
