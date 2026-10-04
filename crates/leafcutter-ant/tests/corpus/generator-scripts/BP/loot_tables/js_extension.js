// js-runtime looks for "<name>.ts" and "<name>.js", so a path that already
// ends in .js is not found.
import helper from '../entities/helper.js'
export default { pools: [], name: helper('x') }
