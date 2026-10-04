const value: number = await Promise.reject(new TypeError('rejected on purpose'))
export default { value }
