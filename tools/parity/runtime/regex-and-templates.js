const re = /[a-z]+\/(?<name>\d+)/giu
const t = `a ${1 + 1} b ${`nested ${re.source}`}`
export { re, t as template }
