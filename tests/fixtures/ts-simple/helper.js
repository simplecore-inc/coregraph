function add(a, b) {
    return a + b;
}

class Calculator {
    constructor() {
        this.history = [];
    }

    compute(op, a, b) {
        const result = op(a, b);
        this.history.push(result);
        return result;
    }
}

module.exports = { add, Calculator };
