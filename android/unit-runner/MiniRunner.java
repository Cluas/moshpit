import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;

/**
 * Runs the {@code @org.junit.Test} methods of the classes named on the command line, each on a fresh instance;
 * prints one line per test and exits 1 if any failed. android/unit.sh's stand-in for JUnit's runner.
 */
public final class MiniRunner {
    public static void main(String[] args) throws Exception {
        int passed = 0, failed = 0;
        for (String name : args) {
            Class<?> type = Class.forName(name);
            for (Method method : type.getDeclaredMethods()) {
                if (!method.isAnnotationPresent(org.junit.Test.class)) continue;
                try {
                    method.invoke(type.getDeclaredConstructor().newInstance());
                    passed++;
                    System.out.println("ok   " + type.getSimpleName() + "." + method.getName());
                } catch (InvocationTargetException e) {
                    failed++;
                    System.out.println("FAIL " + type.getSimpleName() + "." + method.getName() + ": " + e.getCause());
                }
            }
        }
        System.out.println(passed + " passed, " + failed + " failed");
        System.exit(failed == 0 && passed > 0 ? 0 : 1);
    }
}
