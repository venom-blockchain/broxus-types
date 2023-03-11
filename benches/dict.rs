use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use everscale_types::cell::*;
use everscale_types::dict::*;
use rand::distributions::{Distribution, Standard};
use rand::{Rng, SeedableRng};

fn build_dict<C, K, V>(id: BenchmarkId, num_elements: usize, c: &mut Criterion)
where
    for<'c> C: DefaultFinalizer + 'c,
    Standard: Distribution<K> + Distribution<V>,
    K: Store<C> + DictKey,
    V: Store<C>,
{
    let mut rng = rand_xorshift::XorShiftRng::from_seed([0u8; 16]);

    let values = (0..num_elements)
        .map(|_| (rng.gen::<K>(), rng.gen::<V>()))
        .collect::<Vec<_>>();

    c.bench_with_input(id, &values, |b, values| {
        b.iter(|| {
            let mut result = Dict::<C, K, V>::new();
            for (key, value) in values {
                result.set(key, value).unwrap();
            }
            black_box(result);
        });
    });
}

fn build_dict_group<C>(cf: &str, c: &mut Criterion)
where
    for<'c> C: DefaultFinalizer + 'c,
{
    macro_rules! decl_dict_benches {
        ($cf:ident, $({ $n:literal, $k:ty, $v:ident }),*$(,)?) => {
            $({
                let id = BenchmarkId::new(
                    "build_dict",
                    format!(
                        "family={}; size={}; key={}; value={}",
                        cf, $n, stringify!($k), stringify!($v)
                    )
                );
                build_dict::<$cf, $k, $v>(id, $n, c);
            });*
        };
    }

    decl_dict_benches![
        C,

        { 10, u8, u64 },
        { 256, u8, u64 },

        { 10, u16, u64 },
        { 100, u16, u64 },
        { 256, u16, u64 },
        { 10000, u16, u64 },

        { 10, u32, u64 },
        { 100, u32, u64 },
        { 1000, u32, u64 },
        { 100000, u32, u64 },

        { 10, u64, u64 },
        { 100, u64, u64 },
        { 1000, u64, u64 },
        { 100000, u64, u64 },
    ];
}

fn rc_build_dict_group(c: &mut Criterion) {
    build_dict_group::<rc::RcCellFamily>("RcCellFamily", c);
}

fn sync_build_dict_group(c: &mut Criterion) {
    build_dict_group::<sync::ArcCellFamily>("ArcCellFamily", c);
}

criterion_group!(build_dict_rc, rc_build_dict_group);
criterion_group!(build_dict_sync, sync_build_dict_group);
criterion_main!(build_dict_rc, build_dict_sync);
