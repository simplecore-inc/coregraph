export interface Product {
    id: string;
    name: string;
    price: number;
}

export interface Order {
    id: string;
    productId: string;
    quantity: number;
}

export function totalPrice(p: Product, o: Order): number {
    return p.price * o.quantity;
}
