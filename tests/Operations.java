package tests;

public class Operations {
    public static void main(String[] args) {
        intOperations();
    }

    public static void intOperations() {
        {
            int left = 1;
            int right = -99;
            int plus = left + right;
            assert plus == -98;
        }
        {
            int left = 1;
            int right = 99;
            int minus = left - right;
            assert minus == -98;
        }
        {
            int left = 3;
            int right = -9;
            int multiply = left * right;
            assert multiply == -27;
        }
        {
            int left = 10;
            int right = -3;
            int division = left / right;
            assert division == -3;
        }
        {
            int left = 10;
            int right = -3;
            int modulo = left % right;
            assert modulo == 1;
        }
        {
            int left = 2;
            int right = 3;
            int lsh = left << right;
            assert lsh == 16;
        }
        {
            int left = 17;
            int right = 3;
            int rsh = left >> right;
            assert rsh == 2;
        }
        {
            int left = -17;
            int right = 3;
            int rsh = left >>> right;
            assert rsh == 536870909;
        }
    }
}
