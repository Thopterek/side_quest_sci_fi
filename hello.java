/*
 * First compilation with Java
 * name of the file needs to be the same as class name
 * eg. hello -> hello.java
 *
 * while name choosen is hello
 * javac hello.java
 * java hello
*/
public class hello {
  public static void main (String[] ag) {
    System.out.println("Hello World");
    int x = add_operation(3);
    System.out.println("Whats that -> " +x);
    int i = 0;
    while (i != ag.length) {
      System.out.println("This is what was passed to main -> " +ag[i]);
      i++;
    }
    for (int another = 0; another < ag.length; another++)
      System.out.print("" +ag[another]);
  }
  public static int add_operation (int a) {return (a + 1);}
}

