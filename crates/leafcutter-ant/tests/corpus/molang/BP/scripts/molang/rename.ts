import { NameExpression } from '@molang/expressions'

export default (expr: any) => {
	if (expr instanceof NameExpression && expr.toString() === 'v.old') return new NameExpression(expr.env ?? {}, 'v.new')
}
